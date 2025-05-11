//! Types related to task management & Functions for completely changing TCB
use super::schedule::TaskRef;
use super::{pid_alloc, CloneFlags, PidHandle, TaskStatus};
use crate::config::{PAGE_SIZE, PRE_ALLOC_PAGES, USER_STACK_SIZE, USER_STACK_TOP};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{
    put_data, translated_refmut, MapAreaType, MapPermission, MemorySet, PageTable, PageTableEntry,
    VPNRange, VirtAddr, VirtPageNum,
};
use crate::sync::UPSafeCell;
use crate::task::aux::{Aux, AuxType};
use crate::task::kstack::current_stack_top;
use crate::task::processor::UTRAP_HANDLER;
use crate::task::schedule::CFSTask;
use crate::task::{add_task, current_task, PID2PC, TID2TC};
use crate::trap::{TrapContext, TrapStatus};
use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::ManuallyDrop;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicUsize, Ordering};
use core::task::Waker;
use spin::mutex::Mutex;
unsafe impl Sync for ProcessControlBlock {}
unsafe impl Send for ProcessControlBlock {}

/// Task control block structure
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
    pub fd_table: Arc<Mutex<Vec<Option<Arc<dyn File + Send + Sync>>>>>,
    //todo(heliosly)
}

impl ProcessControlBlock {
    pub fn activate_user_memoryset(&self) {
        self.memory_set.lock().activate();
    }
    // ===== user_stack_top =====
    pub fn user_stack_top(&self) -> usize {
        self.user_stack_top.load(Ordering::Acquire)
    }
    pub fn set_user_stack_top(&self, val: usize) {
        self.user_stack_top.store(val, Ordering::Release)
    }

    // ===== is_init =====
    pub fn is_initialized(&self) -> bool {
        self.is_init.load(Ordering::Acquire)
    }
    pub fn set_initialized(&self, val: bool) {
        self.is_init.store(val, Ordering::Release)
    }

    // ===== base_size =====
    pub fn base_size(&self) -> usize {
        self.base_size.load(Ordering::Acquire)
    }
    pub fn set_base_size(&self, val: usize) {
        self.base_size.store(val, Ordering::Release)
    }
    pub fn incr_base_size(&self) -> usize {
        self.base_size.fetch_add(1, Ordering::Relaxed)
    }

    // // ===== cwd =====
    // pub fn cwd(&self) -> String {
    //     let guard = self.cwd.lock();
    //     guard.clone()
    // }
    pub fn set_cwd(&self, path: String) {
        let mut guard = self.cwd.lock();
        *guard = path;
    }

    // ===== parent =====
    pub fn parent(&self) -> usize {
        self.parent.load(Ordering::Acquire)
    }
    pub fn set_parent(&self, p: usize) {
        self.parent.store(p, Ordering::Release)
    }

    // ===== exit_code =====
    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire)
    }
    pub fn set_exit_code(&self, code: i32) {
        self.exit_code.store(code, Ordering::Release)
    }

    // ===== heap_bottom =====
    pub fn heap_bottom(&self) -> usize {
        self.heap_bottom.load(Ordering::Acquire)
    }
    pub fn set_heap_bottom(&self, val: usize) {
        self.heap_bottom.store(val, Ordering::Release)
    }

    // ===== program_brk =====
    pub fn program_brk(&self) -> usize {
        self.program_brk.load(Ordering::Acquire)
    }
    pub fn set_program_brk(&self, val: usize) {
        self.program_brk.store(val, Ordering::Release)
    }

    pub fn alloc_fd(&self) -> usize {
        let mut fd_table = self.fd_table.lock();
        if let Some(fd) = (0..fd_table.len()).find(|fd| fd_table[*fd].is_none()) {
            fd
        } else {
            fd_table.push(None);
            fd_table.len() - 1
        }
    }

    /// Get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.lock().token()
    }

    ///插入一个framed_area
    pub fn insert_framed_area(
        &self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
        area_type: MapAreaType,
    ) {
        self.memory_set
            .lock()
            .insert_framed_area(start_va, end_va, permission, area_type);
    }

    // 根据hint插入页面到指定的area并返回(va_bottom,va_top)
    /// hint指示的区域必须存在
    pub fn insert_framed_area_with_hint(
        &self,
        hint: usize,
        size: usize,
        map_perm: MapPermission,
        area_type: MapAreaType,
    ) -> (usize, usize) {
        let hint_vpn= VirtAddr::from(hint).ceil();
        let npages = (size+ PAGE_SIZE - 1) / PAGE_SIZE;
        let top_vpn = self.memory_set.lock().areatree.find_gap_from(hint_vpn, npages).unwrap();
        let base_vpn = VirtPageNum::from(top_vpn.0-npages);

        let start_va=VirtAddr::from(base_vpn);
        let end_va = VirtAddr::from(top_vpn);
        self.insert_framed_area(
             start_va,
             end_va,
            map_perm,
            area_type,
        );
        (start_va.0, end_va.0)
    }
    ///分配用户资源
    pub fn alloc_user_res(&self) {
        let (ustack_bottom, ustack_top) = self.insert_framed_area_with_hint(
            USER_STACK_TOP,
            USER_STACK_SIZE,
            MapPermission::R | MapPermission::W | MapPermission::U,
            MapAreaType::Stack,
        );

        self.set_user_stack_top(ustack_top);
        
        // let memory_set: spin::MutexGuard<'_, MemorySet> = self.memory_set.lock();
        // assert!(memory_set
        //     .translate(VirtAddr::from(ustack_top).floor())
        //     .unwrap()
        //     .is_valid());

        trace!(
            "user_stack_top:{:#x},bottom:{:#x}",
            ustack_top,
            ustack_bottom
        );
    }
    ///fork
    pub fn clone_user_res(&self, another: &ProcessControlBlock) {
        self.alloc_user_res();
        self.memory_set.lock().clone_area(
            VirtAddr::from(self.user_stack_top() - USER_STACK_SIZE).floor(),
            &another.memory_set.lock(),
        );
    }
    pub fn replace_memory_set(&self, new_ms: MemorySet) {
        let mut guard = self.memory_set.lock();
        let old_ms = core::mem::replace(&mut *guard, new_ms);
        drop(old_ms);
    }
    // pub fn is_exited(&self) -> bool {

    //         let a= *(self.task_status.lock()) ;
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
    //        *self.state_lock().lock() == TaskStatus::Zombie
    //     }
    //  pub fn state_lock(&self) -> &Mutex<TaskStatus> {
    //         &self.task_status
    //     }
    //  #[inline]
    //     /// temp
    //     /// TODO LOCK
    //     pub fn state_lock_manual(&self) -> ManuallyDrop<spin::MutexGuard<'_, TaskStatus>> {
    //                 ManuallyDrop::new(self.task_status.lock())
    //     }
    //  pub fn set_state(&self, state: TaskStatus) {
    //         let mut task_status = self.task_status.lock();
    //         *task_status = state;
    //     }
    //   pub fn get_trap_cx(&self) ->Option<&mut TrapContext >{
    //         unsafe { &mut *self.trap_cx.get() }
    //         .as_mut()
    //         .map(|tf| tf.as_mut())
    // }
}



impl ProcessControlBlock {
    /// Create a new process
    ///
    /// At present, it is only used for the creation of initproc
    pub fn new(elf_data: &[u8], cwd: String, argv: &Vec<String>, env: &mut Vec<String>) -> Self {
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
        )));

        let fd_vec: Vec<Option<Arc<dyn File + Send + Sync>>> = vec![
            Some(Arc::new(Stdin)),
            Some(Arc::new(Stdout)),
            Some(Arc::new(Stdout)),
        ];

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
            tasks: Mutex::new(Vec::new()),
            fd_table: Arc::new(Mutex::new(fd_vec)),
        };
        process_control_block.alloc_user_res();
        let mut user_sp = process_control_block.user_stack_top();
               //环境变量内容入栈
               let mut envp = Vec::new();
               for env in env.iter() {
                   user_sp -= env.len() + 1;
                   envp.push(user_sp);
                   // println!("{:#X}:{}", user_sp, env);
                   for (j, c) in env.as_bytes().iter().enumerate() {
                       *translated_refmut(token, (user_sp + j) as *mut u8) = *c;
                   }
                   *translated_refmut(token, (user_sp + env.len()) as *mut u8) = 0;
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
                       *translated_refmut(token, (user_sp + j) as *mut u8) = *c;
                   }
                   // 添加字符串末尾的 null 字符
                   *translated_refmut(token, (user_sp + arg.len()) as *mut u8) = 0;
               }
               user_sp -= user_sp % size_of::<usize>(); //以8字节对齐
               argvp.push(0);
       
               //需放16个字节
               user_sp -= 16;
               auxv.push(Aux::new(AuxType::RANDOM, user_sp));
               for i in 0..0xf {
                   *translated_refmut(token, (user_sp + i) as *mut u8) = i as u8;
               }
               user_sp -= user_sp % 16;
       
               // println!("aux:");
               //将auxv放入栈中
               auxv.push(Aux::new(AuxType::EXECFN, argvp[0]));
               auxv.push(Aux::new(AuxType::NULL, 0));
               for aux in auxv.iter().rev() {
                   // println!("{:?}", aux);
                   user_sp -= size_of::<Aux>();
                   *translated_refmut(token, user_sp as *mut usize) = aux.aux_type as usize;
                   *translated_refmut(token, (user_sp + size_of::<usize>()) as *mut usize) = aux.value;
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
                   );
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
                   );
               }
       
               //将argc放入栈中
               user_sp -= size_of::<usize>();
               *translated_refmut(token, user_sp as *mut usize) = argv.len();
       
               //以8字节对齐
               user_sp -= user_sp % size_of::<usize>();
               //println!("user_sp:{:#X}", user_sp);
        
       
        // process_control_block.set_user_stack_top(u_sp);
        // prepare TrapContext in user space
        let trap_cx=new_task
            .get_trap_cx()
            .unwrap();
        trap_cx .init(user_sp, entry_point);
        trap_cx.trap_status = TrapStatus::Done;
        trap_cx.regs.a0 = argv.len();
        trap_cx.regs.a1 = argv_base;
        trap_cx.regs.a2 = envp_base;

           
        add_task(new_task.clone());
        TID2TC.lock().insert(new_task.id.0, new_task);
        process_control_block
    }

    /// Load a new elf to replace the original application address space and start execution
    pub fn exec(&self, elf_data: &[u8], argv: &Vec<String>, env: &mut Vec<String>) {
        //用户栈高地址到低地址：环境变量字符串/参数字符串/aux辅助向量/环境变量地址数组/参数地址数组/参数数量
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_heap_base, entry_point, mut auxv) = MemorySet::from_elf(elf_data);

        // **** access current TCB exclusively

        self.replace_memory_set(memory_set);
        self.alloc_user_res();

        let mut user_sp = self.user_stack_top();
        //环境变量内容入栈
        let mut envp = Vec::new();
        let token = self.get_user_token();
        for env in env.iter() {
            user_sp -= env.len() + 1;
            envp.push(user_sp);
            // println!("{:#X}:{}", user_sp, env);
            for (j, c) in env.as_bytes().iter().enumerate() {
                *translated_refmut(token, (user_sp + j) as *mut u8) = *c;
            }
            *translated_refmut(token, (user_sp + env.len()) as *mut u8) = 0;
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
                *translated_refmut(token, (user_sp + j) as *mut u8) = *c;
            }
            // 添加字符串末尾的 null 字符
            *translated_refmut(token, (user_sp + arg.len()) as *mut u8) = 0;
        }
        user_sp -= user_sp % size_of::<usize>(); //以8字节对齐
        argvp.push(0);

        //需放16个字节
        user_sp -= 16;
        auxv.push(Aux::new(AuxType::RANDOM, user_sp));
        for i in 0..0xf {
            *translated_refmut(token, (user_sp + i) as *mut u8) = i as u8;
        }
        user_sp -= user_sp % 16;

        // println!("aux:");
        //将auxv放入栈中
        auxv.push(Aux::new(AuxType::EXECFN, argvp[0]));
        auxv.push(Aux::new(AuxType::NULL, 0));
        for aux in auxv.iter().rev() {
            // println!("{:?}", aux);
            user_sp -= size_of::<Aux>();
            *translated_refmut(token, user_sp as *mut usize) = aux.aux_type as usize;
            *translated_refmut(token, (user_sp + size_of::<usize>()) as *mut usize) = aux.value;
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
            );
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
            );
        }

        //将argc放入栈中
        user_sp -= size_of::<usize>();
        *translated_refmut(token, user_sp as *mut usize) = argv.len();

        //以8字节对齐
        user_sp -= user_sp % size_of::<usize>();
        //println!("user_sp:{:#X}", user_sp);

        //将设置了O_CLOEXEC位的文件描述符关闭 todo(heliosly)
        // update trap_cx ppn

        info!("exec entry_point:{:#x}", entry_point);
        let binding = self.main_task.lock();
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

        // **** release current PCB
    }

    /// parent process fork the child process
    pub fn clone_task(
        &self,
        flags: CloneFlags,
        user_stack: usize,
        ptid: usize,
        tls: usize,
        ctid: usize,
    ) -> isize {
        // ---- hold parent PCB lock
        // alloc a pid and a kernel stack in kernel space

        // copy user space(include trap context)
        let memory_set = if flags.contains(CloneFlags::CLONE_VM) {
            self.memory_set.clone()
        } else {
            Arc::new(Mutex::new(MemorySet::from_existed_user(
                &self.memory_set.lock(),
            )))
        };
        let parent = if flags.contains(CloneFlags::CLONE_PARENT) {
            self.parent()
        } else {
            self.pid.0
        };
        let task_pid_usize: usize;
        let pid = if flags.contains(CloneFlags::CLONE_THREAD) {
            task_pid_usize = self.pid.0;
            None
        } else {
            let a = pid_alloc();
            task_pid_usize = a.0;
            Some(a)
        };

        let fut = UTRAP_HANDLER();
        let tcb = Arc::new(CFSTask::new(TaskControlBlock::new(
            false,
            task_pid_usize,
            memory_set.lock().token(),
            fut,
            Box::new(TrapContext::new()),
        )));
        if flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
            let parent_token = self.memory_set.lock().token();
            *translated_refmut(parent_token, ptid as *mut u32) = tcb.id.0 as u32;
        }
        if flags.contains(CloneFlags::CLONE_SIGHAND) {
            todo!();
            //(Heliosly)
        }
        // 若包含CLONE_CHILD_SETTID或者CLONE_CHILD_CLEARTID
        // 则需要把线程号写入到子线程地址空间中tid对应的地址中
        if flags.contains(CloneFlags::CLONE_CHILD_SETTID)
            || flags.contains(CloneFlags::CLONE_CHILD_CLEARTID)
        {
            todo!();
        }

        //生成线程或者进程
        let res = if flags.contains(CloneFlags::CLONE_THREAD) {
            self.tasks.lock().push(tcb.clone());

            tcb.id.0 as isize
        } else {
            // copy fd table
            let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
            for fd in self.fd_table.lock().iter() {
                if let Some(file) = fd {
                    new_fd_table.push(Some(file.clone()));
                } else {
                    new_fd_table.push(None);
                }
            }

            let process_control_block = Arc::new(ProcessControlBlock {
                pid: pid.unwrap(),
                main_task: Mutex::new(tcb.clone()),

                cwd: Mutex::new(self.cwd.lock().clone()),
                is_init: AtomicBool::new(false),
                base_size: AtomicUsize::new(self.base_size()),
                memory_set,
                parent: AtomicUsize::new(parent),
                children: Mutex::new(Vec::new()),
                exit_code: AtomicI32::new(0),
                fd_table: Arc::new(Mutex::new(new_fd_table)),
                heap_bottom: AtomicUsize::new(self.heap_bottom.load(Ordering::Relaxed)),
                program_brk: AtomicUsize::new(self.program_brk.load(Ordering::Relaxed)),
                user_stack_top: AtomicUsize::new(0),

                tasks: Mutex::new(Vec::new()),
            });

            trace!(
                "[kernel]:clone pid[{}] -> pid[{}]",
                self.pid.0,
                process_control_block.pid.0
            );
            process_control_block.clone_user_res(&self);
            self.children.lock().push(process_control_block.clone());
            PID2PC
                .lock()
                .insert(process_control_block.pid.0, process_control_block.clone());
            process_control_block.pid.0 as isize
        };

        {
            // modify kernel_sp in trap_cx
            let trap_cx = tcb.get_trap_cx().unwrap();
            *trap_cx = current_task().get_trap_cx().unwrap().clone();
            trap_cx.kernel_sp = current_stack_top();
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

        add_task(tcb.clone());
        TID2TC.lock().insert(tcb.id.0, tcb);
        // return
        // **** release child PCB
        // ---- release parent PCB

        res
    }

    /// get pid of process
    pub fn get_pid(&self) -> usize {
        self.pid.0
    }

    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&self, size: i32) -> Option<usize> {
        let heap_bottom = self.heap_bottom();
        let old_break = self.program_brk();
        let new_brk = old_break as isize + size as isize;
        debug!("[BRK] old={:#x}, new={:#x}",  old_break, new_brk);
        if new_brk < heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            self.memory_set
                .lock()
                .shrink_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        } else {
            self.memory_set
                .lock()
                .append_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            self.set_program_brk(new_brk as usize);
            Some(old_break)
        } else {
            None
        }
    }
    ///main task is_zombie
    pub fn is_zombie(&self) -> bool {
        self.main_task.lock().is_exited()
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
    pub state: Mutex<TaskStatus>,
    // time: UnsafeCell<TimeStat>,
    exit_code: AtomicIsize,
    set_child_tid: AtomicUsize,
    clear_child_tid: AtomicUsize,
    /// Whether the task needs to be rescheduled
    ///
    /// When the time slice is exhausted, it needs to be rescheduled
    need_resched: AtomicBool,
    /// The disable count of preemption
    ///
    /// When the task get a lock which need to disable preemption, it
    /// will increase the count. When the lock is released, it will
    /// decrease the count.
    ///
    /// Only when the count is zero, the task can be preempted.
    preempt_disable_count: AtomicUsize,
    /// 在内核中发生抢占或者使用线程接口时的上下文
    // stack_ctx: UnsafeCell<Option<StackCtx>>,

    /// 是否是所属进程下的主线程
    is_leader: AtomicBool,
    process_id: AtomicUsize,
    pub page_table_token: UnsafeCell<usize>,
    // pub cpu_set: AtomicU64,
}
impl TaskControlBlock {
    pub fn new(
        is_init: bool,
        process_id: usize,
        page_table_token: usize,
        fut: Pin<Box<dyn Future<Output = i32> + 'static>>,
        trap_cx: Box<TrapContext>,
    ) -> Self {
        Self {
            id: TaskId::new(),
            is_init,
            exit_code: AtomicIsize::new(0),
            fut: UnsafeCell::new(fut),
            wait_wakers: UnsafeCell::new(VecDeque::new()),
            set_child_tid: AtomicUsize::new(0),
            clear_child_tid: AtomicUsize::new(0),
            need_resched: AtomicBool::new(false),
            preempt_disable_count: AtomicUsize::new(0),
            is_leader: AtomicBool::new(false),
            process_id: AtomicUsize::new(process_id),
            page_table_token: UnsafeCell::new(page_table_token),
            trap_cx: UnsafeCell::new(Some(trap_cx)),
            state: Mutex::new(TaskStatus::Runable),
        }
    }
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
    pub fn state_lock(&self) -> &Mutex<TaskStatus> {
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
}
