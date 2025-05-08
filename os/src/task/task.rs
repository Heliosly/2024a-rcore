//! Types related to task management & Functions for completely changing TCB
use super::schedule::TaskRef;
use super::{ pid_alloc, PidHandle};
use crate::config:: { PRE_ALLOC_PAGES, USER_STACK_SIZE, USER_STACK_TOP};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{ put_data, translated_refmut, MapAreaType, MapPermission, MemorySet, PageTable, PageTableEntry, VPNRange, VirtAddr};
use crate::sync::UPSafeCell;
use crate::task::aux::{self, Aux, AuxType};
use crate::task::kstack::current_stack_top;
use crate::task::processor::UTRAP_HANDLER;
use crate::task::schedule::CFSTask;
use crate::task::PID2PC;
use crate::trap::{    TrapContext, TrapStatus};
use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{RefMut, UnsafeCell};
use core::future::Future;
use core::mem::{replace, ManuallyDrop};
use core::pin::Pin;
use core::task::Waker;
use spin::Mutex;
unsafe impl Sync for ProcessControlBlock{}
unsafe impl Send for ProcessControlBlock {}

/// Task control block structure
///
/// Directly save the contents that will not change during running
pub struct ProcessControlBlock {
 /// The  trapcontext
    pub trap_cx: UnsafeCell<Option<Box<TrapContext>>>,
    // Immutable
    /// Process identifier
    pub pid: PidHandle,


    fut: UnsafeCell<Pin<Box<dyn Future<Output = i32> + 'static>>>,
    /// Mutable
    inner: UPSafeCell<TaskControlBlock>,
/// Maintain the execution status of the current process
    pub task_status: Mutex<TaskStatus>,


}

impl ProcessControlBlock {
pub fn is_zombie(&self) -> bool {
       *self.state_lock().lock() == TaskStatus::Zombie
    }
 /// 获取到任务的 Future
    pub fn get_fut(&self) -> &mut Pin<Box<dyn Future<Output = i32> + 'static>> {
        unsafe { &mut *self.fut.get() }
    }
    /// Get the mutable reference of the inner TCB
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlock> {
        self.inner.exclusive_access()
    }
    /// Get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        let inner = self.inner_exclusive_access();
        inner.memory_set.token()
    }
  
    pub fn set_exit_code(&self, code: i32) {
        let mut inner = self.inner_exclusive_access();
        inner.exit_code = code;
    }   

    pub fn wake_all_waiters(&self){
     let inner = self.inner_exclusive_access();
        inner.wake_all_waiters();

    }
    pub fn is_init(&self) -> bool {
        let inner = self.inner_exclusive_access();
        inner.is_init
    }
    pub fn is_exited(&self) -> bool {
      
        let a= *(self.task_status.lock()) ;
        a== TaskStatus::Zombie

    }
    pub fn get_exit_code(&self) -> i32 {
        let inner = self.inner_exclusive_access();
        inner.exit_code
    }
 pub fn state_lock(&self) -> &Mutex<TaskStatus> {
        &self.task_status
    }
 #[inline]
    /// temp
    /// TODO LOCK
    pub fn state_lock_manual(&self) -> ManuallyDrop<spin::MutexGuard<'_, TaskStatus>> {
                ManuallyDrop::new(self.task_status.lock())
    }
 pub fn set_state(&self, state: TaskStatus) {
        let mut task_status = self.task_status.lock();
        *task_status = state;
    }
  pub fn get_trap_cx(&self) ->Option<&mut TrapContext >{
        unsafe { &mut *self.trap_cx.get() }
        .as_mut()
        .map(|tf| tf.as_mut())
}
}

pub struct TaskControlBlock {
   
    ///trap上下文的bottom
    ///用户栈顶
    pub user_stack_top:usize,
    ///是否初始化
    pub is_init:bool,
    /// Application data can only appear in areas
    /// where the application address space is lower than base_size
    pub base_size: usize,
    /// current work p
    pub cwd: String,

        /// Application address space
    pub memory_set: MemorySet,

    /// Parent process of the current process.
    /// Weak will not affect the reference count of the parent
    pub parent: Option<usize>,

    /// A vector containing TCBs of all child processes of the current process
    pub children: Vec<TaskRef>,

    /// It is set when active exit or execution error occurs
    pub exit_code: i32,
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,

    /// Heap bottom
    pub heap_bottom: usize,

    /// Program break
    pub program_brk: usize,
    ///wait wakers
    pub wait_wakers: UnsafeCell<VecDeque<Waker>>,
}

impl TaskControlBlock {
   
    pub fn wake_all_waiters(&self){
        let wait_wakers = unsafe { &mut *self.wait_wakers.get() };
        while let Some(waker) = wait_wakers.pop_front() {
            waker.wake();
        }
    }
   
 
       ///插入一个framed_area
    pub fn insert_framed_area(&mut self,
            start_va: VirtAddr,
            end_va: VirtAddr,
            permission: MapPermission,
            area_type: MapAreaType,
        ) {
        
             self.memory_set.insert_framed_area(start_va, end_va, permission,area_type); 
        
    }
   
   
   
     // 根据hint插入页面到指定的area并返回(va_bottom,va_top)
    /// hint指示的区域必须存在
    pub fn insert_framed_area_with_hint(
        &mut self,
        hint: usize,
        size: usize,
        map_perm: MapPermission,
        area_type: MapAreaType,
      
    ) -> (usize, usize) {
        let start_va = self.memory_set.
        find_insert_addr(hint, size);
        let end_va = start_va + size;
        self.insert_framed_area(
            VirtAddr::from(start_va),
            VirtAddr::from(end_va),
            map_perm,
            area_type,

           
        );
        (start_va, end_va)
    }
    ///分配用户资源
    pub fn alloc_user_res(&mut self) {
        let (ustack_bottom, ustack_top) = self.insert_framed_area_with_hint(
        USER_STACK_TOP,
            USER_STACK_SIZE,
            MapPermission::R | MapPermission::W | MapPermission::U,
            MapAreaType::Stack,
        );
       
        self.user_stack_top = ustack_top;
        assert!(self.memory_set.translate(crate::mm::VirtPageNum::from(VirtAddr::from(ustack_bottom))).unwrap().is_valid());

        trace!("user_stack_top:{:#x},bottom:{:#x}",ustack_top,ustack_bottom);
        let user_stack_range = VPNRange::new(
            VirtAddr::from(ustack_bottom).floor(),
            VirtAddr::from(ustack_top).floor(),
        );
        //预先为栈顶分配几页，用于环境变量等初始数据
        let page_table = PageTable::from_token(self.memory_set.token());
        let area = self
            .memory_set
            
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.range() == user_stack_range.range())
            .unwrap();
        for i in 1..=PRE_ALLOC_PAGES {
            let vpn = (area.vpn_range.get_end().0 - i).into();
            let pte: Option<PageTableEntry> = page_table.translate(vpn);
            if pte.is_none() || !pte.unwrap().is_valid() {
                area.map_one(&mut self.memory_set.page_table, vpn);
            }
        }
    }
    ///fork
    pub fn clone_user_res(&mut self, another: &TaskControlBlock) {
        self.alloc_user_res();
        self.memory_set.clone_area(
            VirtAddr::from(self.user_stack_top - USER_STACK_SIZE).floor(),
            &another.memory_set,
        );
      
    }
  
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    
    
   
    
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
}

impl ProcessControlBlock {
    ///通过递归调整提示地址，从高到低寻找足够大的未占用虚拟地址空间，确保新分配区域不与现有区域重叠。
    #[inline(always)]
    pub fn insert_framed_area_with_hint(
        &self,
        hint: usize,
        size: usize,
        map_perm: MapPermission,
        area_type: MapAreaType,
    ) -> (usize, usize) {
        self.inner_exclusive_access()
            .insert_framed_area_with_hint(hint, size, map_perm,area_type)
    }
    /// Create a new process
    ///
    /// At present, it is only used for the creation of initproc
    pub fn new(elf_data: &[u8],cwd:String) -> Self {

    // alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        // memory_set with elf program headers/trampoline/trap context/user stack
        // disable_irqs();
        let (memory_set, user_sp, entry_point,_) = MemorySet::from_elf(elf_data);
        // enable_irqs();
        trace!("appenter:{:#x}", entry_point);
        
    
        // push a task context which goes to trap_return to the top of kernel stack
      
        let fut = UTRAP_HANDLER();
        let task_control_block = Self {
            pid: pid_handle,

                    task_status: Mutex::new(TaskStatus::Runable),
            fut: UnsafeCell::new(fut),
   trap_cx:UnsafeCell::new(Some(Box::new(TrapContext::new()))),

            inner: unsafe {
                UPSafeCell::new(TaskControlBlock {
                 
                    base_size: user_sp,
                    memory_set,
                    cwd,
                    is_init:true,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    heap_bottom: user_sp,
                    program_brk: user_sp,
                    user_stack_top:0,
                    wait_wakers:Default::default(),
                })
            },
        };
        let mut task_inner = task_control_block.inner_exclusive_access();
        task_inner.alloc_user_res();
        // prepare TrapContext in user space
       let trap_cx=task_control_block.get_trap_cx().unwrap() ;
       *trap_cx=
            TrapContext::app_init_context(entry_point, task_inner.user_stack_top, 
           );
        trap_cx.kernel_sp = current_stack_top();
        trap_cx.trap_status = TrapStatus::Done;
        drop(task_inner);
        task_control_block

    }

    /// Load a new elf to replace the original application address space and start execution
        pub fn exec(&self, elf_data: &[u8], argv: &Vec<String>, env: &mut Vec<String>) {
            //用户栈高地址到低地址：环境变量字符串/参数字符串/aux辅助向量/环境变量地址数组/参数地址数组/参数数量
        // memory_set with elf program headers/trampoline/trap context/user stack
            let (memory_set, user_heap_base, entry_point,mut auxv) = MemorySet::from_elf(elf_data);
            
        
            // **** access current TCB exclusively
            let mut inner = self.inner_exclusive_access();

            // substitute memory_set
            let old_memory_set = replace(&mut inner.memory_set, memory_set);
            drop(old_memory_set); 
            inner.alloc_user_res();


            let mut user_sp = inner.user_stack_top;
             //环境变量内容入栈
            let mut envp = Vec::new();
            let token =inner.memory_set.token();
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
           let trap_cx= self.get_trap_cx().unwrap() ;
            * trap_cx = TrapContext::app_init_context(
                entry_point,
                user_sp,
            );
            trap_cx.kernel_sp = current_stack_top();
            trap_cx.trap_status = TrapStatus::Done;
            trap_cx.regs.a0 = argv.len();
            trap_cx.regs.a1 = argv_base;
            trap_cx.regs.a2 = envp_base;
            trace!("exec:sp:{:#x}",trap_cx.kernel_sp);
            inner.heap_bottom=user_heap_base;
            inner.program_brk=user_heap_base;
            

            // **** release current PCB
            drop(inner);
        }

        /// parent process fork the child process
    pub fn fork(&self) -> TaskRef {
        // ---- hold parent PCB lock
 // alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        

        let mut parent_inner = self.inner_exclusive_access();
        // copy user space(include trap context)
        let  memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
       
       
        
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent_inner.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        let task_control_block = Arc::new(CFSTask::new(ProcessControlBlock {
            pid: pid_handle,

                    task_status: Mutex::new(TaskStatus::Runable),
                    fut: UnsafeCell::new(UTRAP_HANDLER()),

                    trap_cx:UnsafeCell::new(Some(Box::new(TrapContext::new()))),
            inner: unsafe {
                UPSafeCell::new(TaskControlBlock {
                    

                    cwd:parent_inner.cwd.clone(),
                    is_init:false,
                    wait_wakers:Default::default(),
                    base_size: parent_inner.base_size,
                    memory_set,
                    parent: Some(self.pid.0),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                    user_stack_top:0,
                })
            },
        }));
        // add child
        // modify kernel_sp in trap_cx
        // **** access child PCB exclusively
        

        let mut task_inner= task_control_block.inner_exclusive_access();
        task_inner.clone_user_res(&parent_inner);
        let  trap_cx: &mut TrapContext = task_control_block.get_trap_cx().unwrap();
        *trap_cx = self.get_trap_cx().unwrap().clone();
        trap_cx.kernel_sp = current_stack_top();
        trap_cx.trap_status = TrapStatus::Done;
        trap_cx.regs.a0=0;
        

        parent_inner.children.push(task_control_block.clone());
        PID2PC.lock().insert(task_control_block.pid.0, task_control_block.clone());
        // return
        // **** release child PCB
        // ---- release parent PCB

        drop(task_inner);
        drop(parent_inner);
        
        trace!("[kernel]:fork pid[{}] -> pid[{}]", self.pid.0, task_control_block.pid.0);

        task_control_block
    }

    /// get pid of process
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    
    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner_exclusive_access();
        let heap_bottom = inner.heap_bottom;
        let old_break = inner.program_brk;
        let new_brk = inner.program_brk as isize + size as isize;
        if new_brk < heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            inner
                .memory_set
                .shrink_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        } else {
            inner
                .memory_set
                .append_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            inner.program_brk = new_brk as usize;
            Some(old_break)
        } else {
            None
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Exited, Blocked, Zombie
pub enum TaskStatus {
    Running = 0,
    Runable = 1,
    Blocking = 2,
    Waked = 3,
    Blocked = 4,
    Zombie= 5,
}