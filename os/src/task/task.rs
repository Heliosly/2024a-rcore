//! Types related to task management & Functions for completely changing TCB
use super::fdmanage::FdManage;
use super::schedule::TaskRef;
use super::{current_process, current_token, pid_alloc, yield_now, CloneFlags, PidHandle, ProcessRef, TaskStatus};
use crate::config::{ PAGE_SIZE, USER_STACK_SIZE, USER_STACK_TOP};
use crate::fs::{find_inode, FileClass, FileDescriptor, OpenFlags, Stdin, Stdout};
use crate::mm::{
    put_data, translated_refmut, MapAreaType, MapPermission, MemorySet, VirtAddr, VirtPageNum,
};
use crate::signal::{ProcessSignalSharedState, TaskSignalState};
use crate::syscall::flags::AT_FDCWD;
use crate::task::aux::{Aux, AuxType};
use crate::task::kstack::current_stack_top;
use crate::task::processor::UTRAP_HANDLER;
use crate::task::schedule::CFSTask;
use crate::task::{add_task, current_task, PID2PC, TID2TC};
use crate::trap::{TrapContext, TrapStatus};
use crate::utils::error::{GeneralRet, SysErrNo, SyscallRet};
use crate::utils::string::normalize_absolute_path;
use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::ManuallyDrop;
use core::ops::Deref;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicUsize, Ordering};
use core::task::Waker;
// use spin::mutex::Mutex;

use crate::sync::Mutex;
use spin::mutex::Mutex as Spin;
unsafe impl Sync for ProcessControlBlock {}
unsafe impl Send for ProcessControlBlock {}
///
/// Directly save the contents that will not change during running
pub struct ProcessControlBlock {
    /// The  trapcontext
    // Immutable
    /// Process identifier
    pub pid: PidHandle,
    /// Main task
    pub main_task: Mutex<TaskRef>,
    /// Tasks
    pub tasks: Mutex<Vec<TaskRef>>,
    /// A vector containing TCBs of all child processes of the current process
    pub children: Mutex<Vec<Arc<ProcessControlBlock>>>,

    /// Memeryset
    pub memory_set: Arc<Mutex<MemorySet>>,

    /// trap上下文的bottom
    /// 用户栈顶
    user_stack_top: AtomicUsize,
    /// 是否初始化
    is_init: AtomicBool,
    /// Application data can only appear in areas
    /// where the application address space is lower than base_size
    base_size: AtomicUsize,
    /// current work p
    pub cwd: Mutex<String>,
    pub exe :Mutex<String>,
    /// Parent process of the current process.
    /// Weak will not affect the reference count of the parent
    parent: AtomicUsize,
    /// It is set when active exit or execution error occurs
    exit_code: AtomicI32,
    /// Heap bottom
    heap_bottom: AtomicUsize,
    /// Program break
    program_brk: AtomicUsize,
    /// Application address space
    //     ///wait wakers
    //     pub wait_wakers: UnsafeCell<VecDeque<Waker>>,
    pub fd_table: Arc<Mutex<FdManage>>,
    pub signal_shared_state: Arc<Mutex<ProcessSignalSharedState>>,

    //todo(heliosly)
}
/// `ProcessControlBlock` 的实现。

impl ProcessControlBlock {
    /// 激活用户的内存空间。
    pub async fn activate_user_memoryset(&self) {
        self.memory_set.lock().await.activate();
    }

    /// 获取用户栈顶地址。
    pub fn user_stack_top(&self) -> usize {
        self.user_stack_top.load(Ordering::Acquire)
    }

    /// 设置用户栈顶地址。
    ///
    /// # Arguments
    ///
    /// * `val`: 新的用户栈顶地址。
    pub fn set_user_stack_top(&self, val: usize) {
        self.user_stack_top.store(val, Ordering::Release)
    }

    /// 检查是否初始化。
    pub fn is_initialized(&self) -> bool {
        self.is_init.load(Ordering::Acquire)
    }

    /// 设置初始化状态。
    ///
    /// # Arguments
    ///
    /// * `val`: 初始化状态。
    pub fn set_initialized(&self, val: bool) {
        self.is_init.store(val, Ordering::Release)
    }

    /// 获取基础大小。
    pub fn base_size(&self) -> usize {
        self.base_size.load(Ordering::Acquire)
    }

    /// 设置基础大小。
    ///
    /// # Arguments
    ///
    /// * `val`: 新的基础大小。
    pub fn set_base_size(&self, val: usize) {
        self.base_size.store(val, Ordering::Release)
    }

    /// 增加基础大小。
    pub fn incr_base_size(&self) -> usize {
        self.base_size.fetch_add(1, Ordering::Relaxed)
    }

    /// 设置当前工作目录。
    ///
    /// # Arguments
    ///
    /// * `path`: 新的当前工作目录。
    pub async fn set_cwd(&self, path: String) {
        let mut guard = self.cwd.lock().await;
        *guard = path;
    }

    /// 获取父进程的 ID。
    pub fn parent(&self) -> usize {
        self.parent.load(Ordering::Acquire)
    }

    /// 设置父进程的 ID。
    ///
    /// # Arguments
    ///
    /// * `p`: 父进程的 ID。
    pub fn set_parent(&self, p: usize) {
        self.parent.store(p, Ordering::Release)
    }

    /// 获取退出码。
    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire)
    }

    /// 设置退出码。
    ///
    /// # Arguments
    ///
    /// * `code`: 退出码。
    pub fn set_exit_code(&self, code: i32) {
        self.exit_code.store(code, Ordering::Release)
    }

    /// 获取堆底地址。
    pub fn heap_bottom(&self) -> usize {
        self.heap_bottom.load(Ordering::Acquire)
    }

    /// 设置堆底地址。
    ///
    /// # Arguments
    ///
    /// * `val`: 新的堆底地址。
    pub fn set_heap_bottom(&self, val: usize) {
        self.heap_bottom.store(val, Ordering::Release)
    }

    /// 获取程序 break 地址。
    pub fn program_brk(&self) -> usize {
        self.program_brk.load(Ordering::Acquire)
    }

    /// 设置程序 break 地址。
    ///
    /// # Arguments
    ///
    /// * `val`: 新的程序 break 地址。
    pub fn set_program_brk(&self, val: usize) {
        self.program_brk.store(val, Ordering::Release)
    }

    pub async  fn set_exe(&self,path:String){
        *self.exe.lock().await= path;
    }

    /// 获取应用程序页表的地址。
    pub async fn get_user_token(&self) -> usize {
        self.memory_set.lock().await.token()
    }

    /// 插入一个 framed_area。
    ///
    /// # Arguments
    ///
    /// * `start_va`: 起始虚拟地址。
    /// * `end_va`: 结束虚拟地址。
    /// * `permission`: 内存映射权限。
    /// * `area_type`: 内存区域类型。
    pub async fn insert_framed_area(
        &self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
        area_type: MapAreaType,
    ) {
        self.memory_set
            .lock()
            .await
            .insert_framed_area(start_va, end_va, permission, area_type);
    }

    /// 根据 hint 插入页面到指定的 area 并返回 (va_bottom, va_top)。
    ///
    /// hint 指示的区域必须存在。
    ///
    /// # Arguments
    ///
    /// * `hint`: 区域提示地址。
    /// * `size`: 区域大小。
    /// * `map_perm`: 内存映射权限。
    /// * `area_type`: 内存区域类型。
    ///
    /// # Returns
    ///
    /// * `(usize, usize)`: 分配的虚拟地址范围 (va_bottom, va_top)。
    pub async fn insert_framed_area_with_hint(
        &self,
        hint: usize,
        size: usize,
        map_perm: MapPermission,
        area_type: MapAreaType,
    ) -> (usize, usize) {
        let hint_vpn = VirtAddr::from(hint).ceil();
        let npages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        let base_vpn = self
            .memory_set
            .lock()
            .await
            .areatree
            .find_gap_from(hint_vpn, npages)
            .unwrap();
        let top_vpn = VirtPageNum::from(base_vpn.0 + npages);

        let start_va = VirtAddr::from(base_vpn);
        let end_va = VirtAddr::from(top_vpn);
        self.insert_framed_area(start_va, end_va, map_perm, area_type)
            .await;
        (start_va.0, end_va.0)
    }

    /// 分配用户资源。
    pub async fn alloc_user_res(&self) {
        let (ustack_bottom, ustack_top) = self
            .insert_framed_area_with_hint(
                USER_STACK_TOP,
                USER_STACK_SIZE,
                MapPermission::R | MapPermission::W | MapPermission::U,
                MapAreaType::Stack,
            )
            .await;

        self.set_user_stack_top(ustack_top);

        // let memory_set: spin::MutexGuard<'_, MemorySet> = self.memory_set.lock().await;
        // assert!(memory_set
        //     .translate(VirtAddr::from(ustack_top).floor())
        //     .unwrap()
        //     .is_valid());

        trace!(
            "[alloc_user_stack]user_stack_top:{:#x},bottom:{:#x}",
            ustack_top,
            ustack_bottom
        );
    }

    /// fork
    ///
    /// # Arguments
    ///
    /// * `another`: 另一个 `ProcessControlBlock` 的引用。
    pub async fn clone_user_res(&self, another: &ProcessControlBlock) {
        self.alloc_user_res().await;
        self.memory_set.lock().await.clone_area(
            VirtAddr::from(self.user_stack_top() - USER_STACK_SIZE).floor(),
            another.memory_set.lock().await.deref(),
        );
    }

    /// 替换内存空间。
    ///
    /// # Arguments
    ///
    /// * `new_ms`: 新的 `MemorySet`。
    pub async fn replace_memory_set(&self, new_ms: MemorySet) {
        let mut guard = self.memory_set.lock().await;
        let old_ms = core::mem::replace(&mut *guard, new_ms);
        drop(old_ms);
        let new_token = guard.token();
        for task in self.tasks.lock().await.iter() {
            task.set_token(new_token);
        }
    }
    pub async fn find_task_by_tid(&self, id: usize) -> Option<TaskRef> {
        self.tasks
            .lock()
            .await
            .iter()
            .find(|t| t.id() == id)
            .cloned()
    }
}
//  Non

//         let a= *(self.task_status.lock().await) ;
//         a== TaskStatus::Zombie

//     }
//  /// 获取到任务的 Future
// pub fn get_fut(&self) -> &mut Pin<Box<dyn Future<Output = i32> + 'static>> {
//         unsafe { &mut *self.fut.get() }
//     }
// pub fn wake_all_waiters(&self){
//      let inner = self.inner_exclusive_access();
//         inner.wake_all_waiters();

//     }
// pub fn is_zombie(&self) -> bool {
//        *self.state_lock().await.lock().await == TaskStatus::Zombie
//     }
//  pub fn state_lock(&self) -> &Mutex<TaskStatus> {
//         &self.task_status
//     }
//  #[inline]
//     /// temp
//     pub fn state_lock_manual(&self) -> ManuallyDrop<spin::MutexGuard<'_, TaskStatus>> {
//                 ManuallyDrop::new(self.task_status.lock().await)
//     }
//  pub fn set_state(&self, state: TaskStatus) {
//         let mut task_status = self.task_status.lock().await;
//         *task_status = state;
//     }
//   pub fn get_trap_cx(&self) ->Option<&mut TrapContext >{
//         unsafe { &mut *self.trap_cx.get() }
//         .as_mut()
//         .map(|tf| tf.as_mut())
// }

impl ProcessControlBlock {
    // pub async  fn spawn (
    //     parent: u64,
    //     memory_set: Arc<Mutex<MemorySet>>,
    //     heap_bottom: u64,
    //     fd_table:FdManage ,
    //     cwd: String,
    //     main_task :Mutex<TaskRef>
    //     // mask: Arc<AtomicI32>,
    // )->Self{
    //     let pid =pid_alloc();
    //    let  task_clone =main_task.lock().await.clone();
    //     Self{
    //         pid,
    //         main_task:main_task,
    //         tasks: Mutex::new(vec![task_clone]),
    //         children: Mutex::new(Vec::new()),
    //         memory_set,
    //         user_stack_top: AtomicUsize::new(0),
    //         is_init: AtomicBool::new(false),
    //         base_size: AtomicUsize::new(0),
    //         cwd: Mutex::new(cwd.clone()),
    //         parent: AtomicUsize::new(parent as usize),
    //         exit_code: AtomicI32::new(0),
    //         heap_bottom: AtomicUsize::new(heap_bottom as usize),
    //         program_brk: AtomicUsize::new(0),
    //         fd_table: Arc::new(Mutex::new(fd_table)),

    //         signal_shared_state: Arc::new(Mutex::new(ProcessSignalSharedState::default())),
    //     }

    // }
    /// Create a new process
    ///
    /// At present, it is only used for the creation of initproc
    pub async fn new(
        elf_data: &[u8],
        cwd: String,
        argv: &Vec<String>,
        env: &mut Vec<String>,
        exe: String
    ) -> Self {
        // alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        // memory_set with elf program headers/trampoline/trap context/user stack
        // disable_irqs();
        let (memory_set, user_sp, entry_point, mut auxv) = MemorySet::from_elf(elf_data);
        // enable_irqs();
        trace!("appenter:{:#x}", entry_point);
        let token = memory_set.token();
        // push a task context which goes to trap_return to the top of kernel stack
        let process_id = pid_handle.0;
        let fut = UTRAP_HANDLER();
        let new_task = Arc::new(CFSTask::new(TaskControlBlock::new(
            true,
            process_id,
            token,
            fut,
            Box::new(TrapContext::new()),
            TaskSignalState::default(),
            None,
            false,
        )));

        let fd_vec: Vec<Option<FileDescriptor>> = new_fd_with_stdio();

        let process_control_block = Self {
            pid: pid_handle,
            user_stack_top: AtomicUsize::new(0),
            is_init: AtomicBool::new(true),
            base_size: AtomicUsize::new(user_sp),
            cwd: Mutex::new(cwd),
            parent: AtomicUsize::new(0xDEADBEFF),
            children: Mutex::new(Vec::new()),
            exit_code: AtomicI32::new(0),
            heap_bottom: AtomicUsize::new(user_sp),
            program_brk: AtomicUsize::new(user_sp),
            memory_set: Arc::new(Mutex::new(memory_set)),
            main_task: Mutex::new(new_task.clone()),
            tasks: Mutex::new(vec![new_task.clone()]),
            fd_table: Arc::new(Mutex::new(FdManage(fd_vec))),
            exe: Mutex::new(exe),
            signal_shared_state: Arc::new(Mutex::new(ProcessSignalSharedState::default())),
        };
        process_control_block.alloc_user_res().await;
        let mut user_sp = process_control_block.user_stack_top();
        //环境变量内容入栈
        let mut envp = Vec::new();
        for env in env.iter() {
            user_sp -= env.len() + 1;
            envp.push(user_sp);
            // println!("{:#X}:{}", user_sp, env);
            for (j, c) in env.as_bytes().iter().enumerate() {
                *translated_refmut(token, (user_sp + j) as *mut u8).unwrap() = *c;
            }
            *translated_refmut(token, (user_sp + env.len()) as *mut u8).unwrap() = 0;
        }
        envp.push(0);
        user_sp -= user_sp % size_of::<usize>();

        //存放字符串首址的数组
        let mut argvp = Vec::new();
        for arg in argv.iter() {
            // 计算字符串在栈上的地址
            user_sp -= arg.len() + 1;
            argvp.push(user_sp);
            // println!("{:#X}:{}", user_sp, arg);
            for (j, c) in arg.as_bytes().iter().enumerate() {
                *translated_refmut(token, (user_sp + j) as *mut u8).unwrap() = *c;
            }
            // 添加字符串末尾的 null 字符
            *translated_refmut(token, (user_sp + arg.len()) as *mut u8).unwrap() = 0;
        }
        user_sp -= user_sp % size_of::<usize>(); //以8字节对齐
        argvp.push(0);

        //需放16个字节
        user_sp -= 16;
        auxv.push(Aux::new(AuxType::RANDOM, user_sp));
        for i in 0..0xf {
            *translated_refmut(token, (user_sp + i) as *mut u8).unwrap() = i as u8;
        }
        user_sp -= user_sp % 16;

        // println!("aux:");
        //将auxv放入栈中
        auxv.push(Aux::new(AuxType::EXECFN, argvp[0]));
        auxv.push(Aux::new(AuxType::NULL, 0));
        for aux in auxv.iter().rev() {
            // println!("{:?}", aux);
            user_sp -= size_of::<Aux>();
            *translated_refmut(token, user_sp as *mut usize).unwrap() = aux.aux_type as usize;
            *translated_refmut(token, (user_sp + size_of::<usize>()) as *mut usize).unwrap() =
                aux.value;
        }

        //将环境变量指针数组放入栈中
        // println!("env pointers:");
        user_sp -= envp.len() * size_of::<usize>();
        let envp_base = user_sp;
        for i in 0..envp.len() {
            let _ = put_data(
                token,
                (user_sp + i * size_of::<usize>()) as *mut usize,
                envp[i],
            );
        }

        // println!("arg pointers:");
        user_sp -= argvp.len() * size_of::<usize>();
        let argv_base = user_sp;
        //将参数指针数组放入栈中
        for i in 0..argvp.len() {
            let _ = put_data(
                token,
                (user_sp + i * size_of::<usize>()) as *mut usize,
                argvp[i],
            );
        }

        //将argc放入栈中
        user_sp -= size_of::<usize>();
        *translated_refmut(token, user_sp as *mut usize).unwrap() = argv.len();

        //以8字节对齐
        user_sp -= user_sp % size_of::<usize>();
        //println!("user_sp:{:#X}", user_sp);

        // process_control_block.set_user_stack_top(u_sp);
        // prepare TrapContext in user space
        let trap_cx = new_task.get_trap_cx().unwrap();
        trap_cx.init(user_sp, entry_point);
        trap_cx.trap_status = TrapStatus::Done;
        trap_cx.regs.a0 = argv.len();
        trap_cx.regs.a1 = argv_base;
        trap_cx.regs.a2 = envp_base;

        add_task(new_task.clone());
        TID2TC.lock().insert(new_task.id.0, new_task);
        process_control_block
    }

    /// Load a new elf to replace the original application address space and start execution
    pub async fn exec(
        &self,
        elf_data: &[u8],
        argv: &Vec<String>,
        env: &mut Vec<String>,
    ) -> GeneralRet {
        //用户栈高地址到低地址：环境变量字符串/参数字符串/aux辅助向量/环境变量地址数组/参数地址数组/参数数量
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_heap_base, entry_point, mut auxv) = MemorySet::from_elf(elf_data);

        // **** access current TCB exclusively

        self.replace_memory_set(memory_set).await;
        self.alloc_user_res().await;

        let mut user_sp = self.user_stack_top();
        //环境变量内容入栈
        let mut envp = Vec::new();
        let token = self.get_user_token().await;
        for env in env.iter() {
            user_sp -= env.len() + 1;
            envp.push(user_sp);
            // println!("{:#X}:{}", user_sp, env);
            for (j, c) in env.as_bytes().iter().enumerate() {
                *translated_refmut(token, (user_sp + j) as *mut u8)? = *c;
            }
            *translated_refmut(token, (user_sp + env.len()) as *mut u8)? = 0;
        }
        envp.push(0);
        user_sp -= user_sp % size_of::<usize>();

        //存放字符串首址的数组
        let mut argvp = Vec::new();
        for arg in argv.iter() {
            // 计算字符串在栈上的地址
            user_sp -= arg.len() + 1;
            argvp.push(user_sp);
            // println!("{:#X}:{}", user_sp, arg);
            for (j, c) in arg.as_bytes().iter().enumerate() {
                *translated_refmut(token, (user_sp + j) as *mut u8)? = *c;
            }
            // 添加字符串末尾的 null 字符
            *translated_refmut(token, (user_sp + arg.len()) as *mut u8)? = 0;
        }
        user_sp -= user_sp % size_of::<usize>(); //以8字节对齐
        argvp.push(0);

        //需放16个字节
        user_sp -= 16;
        auxv.push(Aux::new(AuxType::RANDOM, user_sp));
        for i in 0..0xf {
            *translated_refmut(token, (user_sp + i) as *mut u8)? = i as u8;
        }
        user_sp -= user_sp % 16;

        // println!("aux:");
        //将auxv放入栈中
        auxv.push(Aux::new(AuxType::EXECFN, argvp[0]));
        auxv.push(Aux::new(AuxType::NULL, 0));
        for aux in auxv.iter().rev() {
            // println!("{:?}", aux);
            user_sp -= size_of::<Aux>();
            *translated_refmut(token, user_sp as *mut usize)? = aux.aux_type as usize;
            *translated_refmut(token, (user_sp + size_of::<usize>()) as *mut usize)? = aux.value;
        }

        //将环境变量指针数组放入栈中
        // println!("env pointers:");
        user_sp -= envp.len() * size_of::<usize>();
        let envp_base = user_sp;
        for i in 0..envp.len() {
            put_data(
                token,
                (user_sp + i * size_of::<usize>()) as *mut usize,
                envp[i],
            )?;
        }

        // println!("arg pointers:");
        user_sp -= argvp.len() * size_of::<usize>();
        let argv_base = user_sp;
        //将参数指针数组放入栈中
        for i in 0..argvp.len() {
            put_data(
                token,
                (user_sp + i * size_of::<usize>()) as *mut usize,
                argvp[i],
            )?;
        }

        //将argc放入栈中
        user_sp -= size_of::<usize>();
        *translated_refmut(token, user_sp as *mut usize)? = argv.len();

        //以8字节对齐
        user_sp -= user_sp % size_of::<usize>();
        //println!("user_sp:{:#X}", user_sp);

        //TODO(Heliosly)
        // 关闭文件描述符
        //将设置了O_CLOEXEC位的文件描述符关闭 todo(heliosly)
        // update trap_cx ppn
        info!("exec entry_point:{:#x}", entry_point);
        let binding = self.main_task.lock().await;
        let trap_cx: &mut TrapContext = binding.get_trap_cx().unwrap();
        *trap_cx = TrapContext::app_init_context(entry_point, user_sp);
        trap_cx.kernel_sp = current_stack_top();
        trap_cx.trap_status = TrapStatus::Done;
        trap_cx.regs.a0 = argv.len();
        trap_cx.regs.a1 = argv_base;
        trap_cx.regs.a2 = envp_base;
        trace!("exec:sp:{:#x}", trap_cx.kernel_sp);
        self.set_heap_bottom(user_heap_base);
        self.set_program_brk(user_heap_base);
        Ok(())
        // **** release current PCB
    }

    /// parent process fork the child process
    pub async fn clone_task(
        &self,
        flags: CloneFlags,
        user_stack: usize,
        ptid: usize,
        _tls: usize,
        ctid: usize,
    ) -> SyscallRet {
        // ---- hold parent PCB lock
        // alloc a pid and a kernel stack in kernel space

        // copy user space(include trap context)
        // 子线程或者进程的memory_set和 clone_token
        let (memory_set,clone_token )= if flags.contains(CloneFlags::CLONE_THREAD) {
            (None,self.get_user_token().await )
        } else {
            let ms=if flags.contains(CloneFlags::CLONE_VM) {
                self.memory_set.clone()
            } else {
                Arc::new(Mutex::new(MemorySet::from_existed_user(
                    &mut *self.memory_set.lock().await,
                )))
            };
            let token = ms.lock().await.token();
            (Some(ms),token)
        };
        let parent = if flags.contains(CloneFlags::CLONE_PARENT) {
            self.parent()
        } else {
            self.pid.0
        };
        let new_sig_state = TaskSignalState::init(current_task().signal_state.lock().await.sigmask);

        let task_pid_usize: usize;
        let pid = if flags.contains(CloneFlags::CLONE_THREAD) {
            task_pid_usize = self.pid.0;
            None
        } else {
            let pidhandle = pid_alloc();
            task_pid_usize = pidhandle.0;
            Some(pidhandle)
        };

        // 若包含CLONE_CHILD_SETTID或者CLONE_CHILD_CLEARTID
        // 则需要把线程号写入到子线程地址空间中tid对应的地址中
        let (child_tid,need_set_tid) = if flags.contains(CloneFlags::CLONE_CHILD_SETTID)
            || flags.contains(CloneFlags::CLONE_CHILD_CLEARTID)
        {
                assert!(ctid!=0);

            
            if flags.contains(CloneFlags::CLONE_CHILD_SETTID){
                  (Some(ctid),true)
            }
            else{
                 (Some(ctid),false)
            }
            
        } 
        else{
            (None,false)
        };
        
        
        let trap_cx = Box::new(*current_task().get_trap_cx().unwrap());

        let fut = UTRAP_HANDLER();
        let tcb = Arc::new(CFSTask::new(TaskControlBlock::new(
            false,
            task_pid_usize,
            clone_token,
            fut,
            trap_cx,
            new_sig_state,
            child_tid,
            need_set_tid,
            
        )));
        if flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
            let parent_token = self.memory_set.lock().await.token();
            *translated_refmut(parent_token, ptid as *mut u32)? = tcb.id.0 as u32;
        }

        // trace!("flags:{:#?}",flags);
        //生成线程或者进程
        let res = if flags.contains(CloneFlags::CLONE_THREAD) {
            self.tasks.lock().await.push(tcb.clone());

            tcb.id.0
        } else {
            let new_proc_sig_state = if flags.contains(CloneFlags::CLONE_SIGHAND) {
                current_process().signal_shared_state.clone()
            } else {
                Arc::new(Mutex::new(ProcessSignalSharedState::clone_from_another(
                    &*current_process().signal_shared_state.lock().await,
                )))
            };
            // copy fd table
            let mut new_fd_table: Vec<Option<FileDescriptor>> = Vec::new();
            for fd in self.fd_table.lock().await.0.iter() {
                if let Some(file) = fd {
                    new_fd_table.push(Some(file.clone())); //todo()
                } else {
                    new_fd_table.push(None);
                }
            }

            let process_control_block = Arc::new(ProcessControlBlock {
                pid: pid.unwrap(),
                main_task: Mutex::new(tcb.clone()),

                cwd: Mutex::new(self.cwd.lock().await.clone()),
                is_init: AtomicBool::new(false),
                base_size: AtomicUsize::new(self.base_size()),
                memory_set:memory_set.unwrap(),
                parent: AtomicUsize::new(parent),
                children: Mutex::new(Vec::new()),
                exit_code: AtomicI32::new(0),
                fd_table: Arc::new(Mutex::new(FdManage(new_fd_table))),
                heap_bottom: AtomicUsize::new(self.heap_bottom.load(Ordering::Relaxed)),
                program_brk: AtomicUsize::new(self.program_brk.load(Ordering::Relaxed)),
                user_stack_top: AtomicUsize::new(0),

                signal_shared_state: new_proc_sig_state,
                tasks: Mutex::new(Vec::new()),
                exe:Mutex::new(self.exe.lock().await.clone()),
            });
            if user_stack == 0 {
                if !flags.contains(CloneFlags::CLONE_VM) {
                    process_control_block.clone_user_res(self).await;
                }
            }
            trace!(
                "[kernel]:clone pid[{}] -> pid[{}]",
                self.pid.0,
                process_control_block.pid.0
            );
            process_control_block.tasks.lock().await.push(tcb.clone());
            // process_control_block.clone_user_res(&self);
            self.children
                .lock()
                .await
                .push(process_control_block.clone());
            PID2PC
                .lock()
                .insert(process_control_block.pid.0, process_control_block.clone());
            process_control_block.pid.0
        };

        {
            let trap_cx = tcb.get_trap_cx().unwrap();
            // modify kernel_sp in trap_cx
            trap_cx.trap_status = TrapStatus::Done;
            trap_cx.regs.a0 = 0;

            // 设置用户栈
            // 若给定了用户栈，则使用给定的用户栈
            // 若没有给定用户栈，则使用当前用户栈
            // 没有给定用户栈的时候，只能是共享了地址空间，且原先调用clone的有用户栈，此时已经在之前的trap clone时复制了
            if user_stack != 0 {
                trap_cx.set_sp(user_stack);

                // info!(
                //     "New user stack: sepc:{:X}, stack:{:X}",
                //     trap_frame.sepc, trap_frame.regs.sp
                // );
            }
        }
        if flags.contains(CloneFlags::CLONE_CHILD_SETTID){
            *translated_refmut(clone_token, ctid as *mut u32)? = tcb.id() as u32;
        }







        add_task(tcb.clone());
        TID2TC.lock().insert(tcb.id.0, tcb);
        // return
        // **** release child PCB
        // ---- release parent PCB
        yield_now().await;

        Ok(res)
    }

    /// get pid of process
    pub fn get_pid(&self) -> usize {
        self.pid.0
    }

    /// change the location of the program break. return None if failed.
    pub async fn change_program_brk(&self, new_brk: usize) -> Option<usize> {
        let heap_bottom = self.heap_bottom();
        let old_break = self.program_brk();
        let size = new_brk as isize - old_break as isize;
        debug!("[BRK] old={:#x}, new={:#x}", old_break, new_brk);
        if new_brk < heap_bottom {
            return None;
        }
        let result = if size < 0 {
            self.memory_set
                .lock()
                .await
                .shrink_to(VirtAddr(heap_bottom), VirtAddr(new_brk))
        } else {
            self.memory_set
                .lock()
                .await
                .append_to(VirtAddr(heap_bottom), VirtAddr(new_brk))
        };
        if result {
            self.set_program_brk(new_brk);
            Some(new_brk)
        } else {
            None
        }
    }
    ///main task is_zombie
    pub async fn is_zombie(&self) -> bool {
        self.main_task.lock().await.is_exited()
    }
    /// 解析路径，处理 dirfd、相对/绝对路径、"."、".." 和符号链接。
    /// 利用底层 VFS 的 find 方法（它已经处理了符号链接递归）。
    /// 会持有fd_table的锁
    /// # 参数
    /// * `proc_arc`: 当前进程的 `Arc<ProcessControlBlock>`。
    /// * `dirfd`: 目录文件描述符，或 AT_FDCWD。
    /// * `path_str`: 要解析的路径字符串 (已在内核空间)。
    /// * `follow_last_symlink`: 是否解析路径中最后一个组件如果是符号链接。
    ///
    /// # 返回
    /// `Result<String, SysErrNo>`: 成功时为最终的、规范化的绝对路径，失败时为错误码。
    pub async fn resolve_path_from_fd(
        &self,
        dirfd: i32,
        path_str: &str,
        follow_last_symlink: bool,
    ) -> Result<String, SysErrNo> {
        log::trace!(
            "resolve_path_from_fd: dirfd={}, path='{}', follow_last={}",
            dirfd, path_str, follow_last_symlink
        );

        // 1. 确定基准绝对路径 (base_path_string)
        let base_path_string: String;
        if path_str.starts_with('/') {
            base_path_string = "/".to_string();
        } else if dirfd == AT_FDCWD {
            base_path_string = self.cwd.lock().await.clone();
        } else {
            let fd_table_guard = &self.fd_table.lock().await.0;
            let dir_file_desc_opt = fd_table_guard
                .get(dirfd as usize)
                .and_then(|opt| opt.as_ref());
            match dir_file_desc_opt {
                Some(dir_file_desc) => {
                    // 从 FileDescriptor 获取其代表的 VfsNodeOps
                    let dir_vfs_node = dir_file_desc.file()?;
                    if !dir_vfs_node.is_dir() {
                        // VfsNodeOps 需要 is_dir()
                        return Err(SysErrNo::ENOTDIR);
                    }
                    base_path_string = dir_vfs_node.get_path();
                }
                None => return Err(SysErrNo::EBADF),
            }
        }

        // 2. 拼接路径并进行初步的字符串规范化 (处理 ., .., //)
        let initial_combined_path = if path_str.starts_with('/') {
            path_str.to_string() // 如果已经是绝对路径，则不与 base_path 拼接
        } else {
            let mut combined = base_path_string;
            if !combined.ends_with('/') && !path_str.is_empty() {
                combined.push('/');
            }
            combined.push_str(path_str);
            combined
        };
        let normalized_path_to_find = normalize_absolute_path(&initial_combined_path);
        // log::trace!("Path to find after normalization: {}", normalized_path_to_find);

        // 3. 准备传递给 VFS `find` 方法的 OpenFlags
        let find_flags = OpenFlags::empty(); // 或者一个基础的查找模式，如 O_PATH
        if follow_last_symlink {
            match find_inode(&normalized_path_to_find, find_flags) {
                Ok(found_vfs_node) => {
                    // 5. 从找到的 VfsNodeOps 获取其最终的绝对路径
                    return Ok(found_vfs_node.path()); // 返回 Result<String, SysErrNo>
                }
                Err(SysErrNo::ENOENT) => {
                    // 如果 find 返回 ENOENT，但我们不 follow 最后一个符号链接，
                    // 并且原始路径（规范化后）的父目录存在，
                    // 那么结果应该是这个符号链接本身的路径（如果它存在的话）。
                    //  find 方法在 O_ASK_SYMLINK 时，如果最后一个是符号链接，会直接返回它。
                    // 所以，如果到这里是 ENOENT，意味着路径（或其前缀）确实不存在。
                    return Err(SysErrNo::ENOENT);
                }
                Err(e) => return Err(e),
            }
        }
        // faccessat 本身不打开文件，我们传递的 flags 主要是为了控制符号链接行为。

        // info!("[resolve_path_from_fd] normal_path_str: {}",normalized_path_to_find);
        Ok(normalized_path_to_find)
    }
}

/// A unique identifier for a thread.
pub struct TaskId(usize);

static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
impl TaskId {
    /// Create a new task ID.
    pub fn new() -> Self {
        Self(ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Convert the task ID to a `u64`.
    pub const fn as_usize(&self) -> usize {
        self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for TaskControlBlock {}
unsafe impl Sync for TaskControlBlock {}

pub struct TaskControlBlock {
    fut: UnsafeCell<Pin<Box<dyn Future<Output = i32> + 'static>>>,
    trap_cx: UnsafeCell<Option<Box<TrapContext>>>,

    // executor: SpinNoIrq<Arc<Executor>>,
    pub wait_wakers: UnsafeCell<VecDeque<Waker>>,
    // pub scheduler: SpinNoIrq<Arc<SpinNoIrq<Scheduler>>>,
    pub id: TaskId,
    /// Whether the task is the initial task
    ///
    /// If the task is the initial task, the kernel will terminate
    /// when the task exits.
    pub is_init: bool,
    pub state: Spin<TaskStatus>,
    // time: UnsafeCell<TimeStat>,
    exit_code: AtomicIsize,
    /// Whether the task needs to be rescheduled
    ///
    /// When the time slice is exhausted, it needs to be rescheduled
    _need_resched: AtomicBool,
    /// The disable count of preemption
    ///
    /// When the task get a lock which need to disable preemption, it
    /// will increase the count. When the lock is released, it will
    /// decrease the count.
    ///
    /// Only when the count is zero, the task can be preempted.
    _preempt_disable_count: AtomicUsize,
    /// 在内核中发生抢占或者使用线程接口时的上下文
    // stack_ctx: UnsafeCell<Option<StackCtx>>,
    /// 是否是所属进程下的主线程
    is_leader: AtomicBool,
    process_id: AtomicUsize,
    pub signal_state: Mutex<TaskSignalState>,
    pub page_table_token: UnsafeCell<usize>,
    /// bool位表示是否需要clear
    pub child_tid_ptr: Option<usize>,
    pub need_clear_child_tid:AtomicBool,
     
    // pub cpu_set: AtomicU64,
}
impl TaskControlBlock {
    pub fn new(
        is_init: bool,
        process_id: usize,
        page_table_token: usize,
        fut: Pin<Box<dyn Future<Output = i32> + 'static>>,
        trap_cx: Box<TrapContext>,
        signal_state: TaskSignalState,
        chlid_tid_ptr: Option<usize>,
        clear_child_tid:bool,
    ) -> Self {
        Self {
            id: TaskId::new(),
            is_init,
            exit_code: AtomicIsize::new(0),
            fut: UnsafeCell::new(fut),
            wait_wakers: UnsafeCell::new(VecDeque::new()),
            _need_resched: AtomicBool::new(false),
            _preempt_disable_count: AtomicUsize::new(0),
            is_leader: AtomicBool::new(false),
            process_id: AtomicUsize::new(process_id),
            page_table_token: UnsafeCell::new(page_table_token),
            trap_cx: UnsafeCell::new(Some(trap_cx)),
            state: Spin::new(TaskStatus::Runnable),
            signal_state: Mutex::new(signal_state),
            child_tid_ptr:chlid_tid_ptr,
            need_clear_child_tid: AtomicBool::new(clear_child_tid),
            
        }
    }
    ///
    pub fn id(&self) -> usize {
        self.id.0
    }
    ///
    pub fn get_tid(&self) -> usize {
        self.id.0
    }
    /// 设置任务状态。
    pub fn set_state(&self, state: TaskStatus) {
        let mut task_status = self.state.lock();
        *task_status = state;
    }
    pub fn wake_all_waiters(&self) {
        let wait_wakers = unsafe { &mut *self.wait_wakers.get() };
        while let Some(waker) = wait_wakers.pop_front() {
            waker.wake();
        }
    }
    /// 获取到任务的 Future
    pub fn get_fut(&self) -> &mut Pin<Box<dyn Future<Output = i32> + 'static>> {
        unsafe { &mut *self.fut.get() }
    }

    pub fn set_exit_code(&self, code: isize) {
        self.exit_code.store(code, Ordering::Relaxed);
    }

    pub fn clear_child_tid(&self)->GeneralRet{
        if self.need_clear_child_tid.load(Ordering::Acquire){
            *translated_refmut(unsafe { *self.page_table_token.get() }, self.child_tid_ptr.unwrap() as *mut u32)?=0;
        }
        self.need_clear_child_tid.store(false, Ordering::Relaxed);
        Ok(())
    }
    pub fn is_exited(&self) -> bool {
        *(self.state.lock()) == TaskStatus::Zombie
    }
    pub fn get_exit_code(&self) -> isize {
        self.exit_code.load(Ordering::Relaxed)
    }
    #[inline]
    /// temp
    /// TODO LOCK
    pub fn state_lock_manual(&self) -> ManuallyDrop<spin::MutexGuard<'_, TaskStatus>> {
        ManuallyDrop::new(self.state.lock())
    }

    pub fn get_trap_cx(&self) -> Option<&mut TrapContext> {
        unsafe { &mut *self.trap_cx.get() }
            .as_mut()
            .map(|tf| tf.as_mut())
    }
    pub fn state_lock(&self) -> &spin::Mutex<TaskStatus> {
        &self.state
    }
    pub fn is_zombie(&self) -> bool {
        *self.state_lock().lock() == TaskStatus::Zombie
    }
    pub fn get_pid(&self) -> usize {
        self.process_id.load(Ordering::Relaxed)
    }
    pub fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::Relaxed)
    }
    pub fn set_token(&self, token: usize) {
        unsafe {
            *self.page_table_token.get() = token;
        }
    }

    pub fn get_process(&self) -> ProcessRef {
        Arc::clone(PID2PC.lock().get(&self.get_pid()).unwrap())
    }
}

pub fn new_fd_with_stdio() -> Vec<Option<FileDescriptor>> {
    vec![
        Some(FileDescriptor {
            flags: OpenFlags::O_RDONLY,
            file: FileClass::Abs(Arc::new(Stdin)),
        }),
        // stdout: 只写
        Some(FileDescriptor {
            flags: OpenFlags::O_WRONLY,
            file: FileClass::Abs(Arc::new(Stdout)),
        }),
        // stderr: 只写
        Some(FileDescriptor {
            flags: OpenFlags::O_WRONLY,
            file: FileClass::Abs(Arc::new(Stdout)),
        }),
    ]
}
