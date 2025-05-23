//!Implementation of [`Processor`] and Intersection of control flow
//!
//! Here, the continuous operation of user apps in CPU is maintained,
//! the current running state of CPU is recorded,
//! and the replacement and transfer of control flow of different applications are executed.

use super::current::CurrentTask;
use super::task::TaskControlBlock;
use super::{schedule, TaskStatus};
use crate::mm::activate_by_token;
use crate::task::kstack::{self, current_stack_bottom, current_stack_top};
use crate::task::sleeplist::init_sleeper_queue;
use crate::task::{put_prev_task };
use crate::trap::{disable_irqs, enable_irqs, user_return, TrapContext, TrapStatus};
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::Future;
use core::panic;
use core::pin::Pin;
use core::task::{Context, Poll};
use lazy_init::LazyInit;
use schedule::CFScheduler;
use spin::mutex::Mutex as Spin;
/// Processor management structure

pub static KERNEL_SCHEDULER: LazyInit<Arc<Spin<CFScheduler<TaskControlBlock>>>> = LazyInit::new();
pub static UTRAP_HANDLER: LazyInit<fn() -> Pin<Box<dyn Future<Output = i32> + 'static>>> =
    LazyInit::new();

///runtask with future;
/// IMPO
pub fn run_task2(mut curr: CurrentTask) {
    let waker = curr.waker();
    let cx = &mut Context::from_waker(&waker);
    activate_by_token(unsafe { *curr.page_table_token.get() });

    //delete active(conflict)

    // 拿到 future 的所有权，而不是引用！

    // debug!("poll");
    let res = curr.get_fut().as_mut().poll(cx);
    // debug!("polled task res:{:#?}", res);
    match res {
        Poll::Ready(exit_code) => {
            debug!("task exit: todo, exit_code={}", exit_code);
            curr.set_state(TaskStatus::Zombie);
            curr.set_exit_code(exit_code as isize);
            curr.wake_all_waiters();
            // println!("count {}",Arc::strong_count(curr.as_task_ref()));
            if curr.is_init {
                assert!(
                    Arc::strong_count(curr.as_task_ref()) <= 2,
                    "count {}",
                    Arc::strong_count(curr.as_task_ref())
                );
                panic!("shotdown");
            }
            CurrentTask::clean_current();
            // trace!("current task is cleared1");
        }
        Poll::Pending => {
            let mut state = curr.state_lock_manual();

            trace!("res is pending and Taskstatus:{}", **state as usize);
            match **state {
                // await 主动让权，将任务的状态修改为就绪后，放入就绪队列中
                TaskStatus::Running => {
                    if let Some(tf) = curr.get_trap_cx() {
                        if tf.trap_status == TrapStatus::Done {
                            tf.kernel_sp = kstack::current_stack_top();
                            tf.scause = 0;
                            // 这里不能打开中断
                            disable_irqs();
                            drop(core::mem::ManuallyDrop::into_inner(state));

                            trace!(
                                "user return return val: {} sepc:{:#x},u_sp:{:#x},pid:{}",
                                tf.regs.a0,
                                tf.sepc,
                                tf.regs.sp,
                                curr.get_pid()
                            );
                            // trace!("ret tf");
                            //  trace!(
                            //                                 "return trapcontext :{:#?}",
                            //                                 tf
                            //                             );
                            enable_irqs();
                            user_return(tf);
                        }
                    }

                    **state = TaskStatus::Runnable;
                    put_prev_task(curr.clone());
                    CurrentTask::clean_current();
                }

                // 处于 Runable 状态的任务一定处于就绪队列中，不可能在 CPU 上运行
                TaskStatus::Runnable => panic!("Runable ? cannot be peding"),
                // 等待 Mutex 等进入到 Blocking 状态，但还在这个 CPU 上运行，
                // 此时还没有被唤醒，因此将状态修改为 Blocked，等待被唤醒
                TaskStatus::Blocking => {
                    trace!("current task is clean without drop");
                    **state = TaskStatus::Blocked;
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
                TaskStatus::Zombie => panic!("Exited cannot be pending"),
            }
            // 在这里释放锁，中间的过程不会发生中断
            drop(core::mem::ManuallyDrop::into_inner(state));
        }
    }
}
//Return to idle control flow for new scheduling
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
    info!("current kernel stack top:{:#x}", current_stack_top());

    info!("current kernel stack bottom:{:#x}", current_stack_bottom());
    // kstack::alloc_current_stack();
    UTRAP_HANDLER.init_by(utrap_handler);
    let scheduler = CFScheduler::new();
    KERNEL_SCHEDULER.init_by(Arc::new(Spin::new(scheduler)));
    // let task = Arc::new(CFSTask::new(TaskControlBlock::new(
    //     false,
    //     1,
    //     *KERNEL_PAGE_TABLE_TOKEN,
    //     Box::pin(async move {
    //         debug!("[idle kernel thread] ");
    //         0xdead as i32
    //     }),
    //     Box::new(TrapContext::new()),
    // )));
    // // unsafe { CurrentTask::init_current(task.clone()) };
    // let pcb = Arc::new(ProcessControlBlock::spawn(
    //     0,
    //     KERNEL_SPACE.clone(),
    //     0,
    //     new_fd_with_stdio(),
    //     "".into(),
    //     Mutex::new(task),
    // ));
    // KERNEL_IDLE_PROCESS.init_by(pcb);
    init_sleeper_queue();
}
