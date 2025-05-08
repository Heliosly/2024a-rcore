//!Implementation of [`Processor`] and Intersection of control flow
//!
//! Here, the continuous operation of user apps in CPU is maintained,
//! the current running state of CPU is recorded,
//! and the replacement and transfer of control flow of different applications are executed.

use core::future::Future;
use core::panic;
use core::pin::Pin;
use core::task::{Context, Poll};
use spin::mutex::Mutex;
use schedule::CFScheduler;
use super::current::CurrentTask;
use super::{schedule,  TaskStatus};
use super:: TaskControlBlock;
use crate::sync::UPSafeCell;
use crate::task::kstack::{self, current_stack_bottom, current_stack_top};
use crate::task::put_prev_task;
use crate::trap::{ disable_irqs, enable_irqs, user_return,  TrapStatus}  ;
use alloc::boxed::Box;
use alloc::sync::Arc;
use lazy_init::LazyInit;
use lazy_static::*;
/// Processor management structure
pub struct Processor {
    ///The task currently executing on the current processor
    current: Option<Arc<TaskControlBlock>>,

}
impl Processor {
    pub fn set_current(&mut self, task: Arc<TaskControlBlock>) {
        self.current = Some(task);
    }
    ///Create an empty Processor
    pub fn new() -> Self {
        Self {
            current: None,
        }
    }

    //Get mutable reference to `idle_task_cx`
    

    ///Get current task in moving semanteme
    pub fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }

    ///Get current task in cloning semanteme
    pub fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }
    ///Get current task ref
    pub fn current_ref(&self) -> Option<&Arc<TaskControlBlock>> {
            self.current.as_ref()
        }
}

lazy_static! {
    pub static ref PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };
}
pub static KERNEL_SCHEDULER: LazyInit<Arc<Mutex<CFScheduler<TaskControlBlock>>>> = LazyInit::new();
pub static UTRAP_HANDLER: LazyInit<fn() -> Pin<Box<dyn Future<Output = i32> + 'static>>> =
    LazyInit::new();
///The main part of process execution and scheduling
///Loop `fetch_task` to get the process that needs to run, and switch the process through `__switch`
pub fn run_tasks() {
    loop {
        panic!("undo");
    }
}
///runtask with future;
/// IMPO
pub fn run_task2(mut curr:CurrentTask){
    let waker = curr.waker();
    let cx = &mut Context::from_waker(&waker);
        let  inner = curr.inner_exclusive_access();
        
       
        inner.memory_set.activate();
    
        drop(inner);
        // 拿到 future 的所有权，而不是引用！

    debug!("poll");
        let res = curr.get_fut().as_mut().poll(cx);
        debug!("polled task res:{:#?}",res);
    match res {
        Poll::Ready(exit_code) => {
            debug!("task exit: todo, exit_code={}", exit_code);
            curr.set_state(TaskStatus::Zombie);
            curr.set_exit_code(exit_code);
            curr.wake_all_waiters();
            if curr.is_init() {
                assert!(
                    Arc::strong_count(curr.as_task_ref()) <= 3,
                    "count {}",
                    Arc::strong_count(curr.as_task_ref())
                );
                panic!("shotdown");
            }
            CurrentTask::clean_current();
            // trace!("current task is cleared1");
        }
        Poll::Pending => {
            let mut state= curr.state_lock_manual();

    trace!("res is pending and Taskstatus:{}",**state as usize);
            match **state {
                // await 主动让权，将任务的状态修改为就绪后，放入就绪队列中
                TaskStatus::Running => {
                    if let Some(tf) = curr.get_trap_cx(){
                        if tf.trap_status == TrapStatus::Done {
                            
                            tf.kernel_sp = kstack::current_stack_top();
                            tf.scause = 0;
                            // 这里不能打开中断
                            disable_irqs();
                            drop(core::mem::ManuallyDrop::into_inner(state));
                            
                            trace!("user return return val: {} sepc:{:#x}",tf.regs.a0,tf.sepc);
                            
                            enable_irqs();
                            user_return(tf);  
                            
                        }      
                    }
                       
                            **state = TaskStatus::Runable;
                            put_prev_task(curr.clone());
                            CurrentTask::clean_current();

                            
                  
                           
                    }
                    
                   
                    

            
                
                // 处于 Runable 状态的任务一定处于就绪队列中，不可能在 CPU 上运行
                TaskStatus::Runable => panic!("Runable ? cannot be peding"),
                // 等待 Mutex 等进入到 Blocking 状态，但还在这个 CPU 上运行，
                // 此时还没有被唤醒，因此将状态修改为 Blocked，等待被唤醒

                TaskStatus::Blocking => {
                    
            trace!("current task is clean without drop");
            **state = TaskStatus    ::Blocked;
            CurrentTask::clean_current_without_drop();
                }
                // 由于等待 Mutex 等，导致进入到了 Blocking 状态，但在这里还没有修改状态为 Blocked 时
                // 已经被其他 CPU 上运行的任务唤醒了，因此这里直接返回，让当前的任务继续执行
                TaskStatus::Waked => {
                    **state = TaskStatus::Running;
                }
                // Blocked 状态的任务不可能在 CPU 上运行
                TaskStatus::Blocked => panic!("Blocked  cannot be pending"),
                // 退出的任务只能对应到 Poll::Ready
                TaskStatus::Zombie=> panic!("Exited cannot be pending"),
            }
            // 在这里释放锁，中间的过程不会发生中断
            drop(core::mem::ManuallyDrop::into_inner(state));
        }
    }
}
///Return to idle control flow for new scheduling
// pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
//     let mut processor = PROCESSOR.exclusive_access();
//     let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
//     drop(processor);
//     unsafe {
//         __switch(switched_task_cx_ptr, idle_task_cx_ptr);
//     }
// }
// Initializes the Processor
pub fn init(utrap_handler: fn() -> Pin<Box<dyn Future<Output = i32> + 'static>>) {
    info!("Initialize executor...");
    kstack::init();
    info!("current kernel stack top:{:#x}",current_stack_top());

    info!("current kernel stack bottom:{:#x}",current_stack_bottom());
    // kstack::alloc_current_stack();
    UTRAP_HANDLER.init_by(utrap_handler);
    let scheduler = CFScheduler::new();
    KERNEL_SCHEDULER.init_by(Arc::new(Mutex::new(scheduler)));
}





