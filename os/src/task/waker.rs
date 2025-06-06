//! 这个模块手动构造 vTable，来构建 Waker
//! 构建过程中不会对 TaskRef 的引用计数增加
//! 因此，在定时器或者等待队列中注册的 Waker 不会增加引用计数
//! 从而不会产生由于 Arc 引用计数导致的性能开销
//! 为了保证 Waker 中的指针有效，需要保证 TaskRef 不会被释放
//! 这里使用的技巧是在 run_future 是：
//! 1. 若 task 返回 Ready，则会释放掉这个任务
//! 2. 若 task 返回 Pending，会调用 CurrentTask::clean_current_without_drop
//!    不释放 TaskRef，一直到 TaskRef 执行返回 Ready，将其清空，才会被释放
//!
//! 这种做法保证了 Task 模块内的代码，只有在创建时才会对引用计数增加
//! 不会因为任务阻塞而导致引用计数增加，
//! 其余对 TaskRef 引用计数的操作只会源于其余模块中的操作

use core::{ task::{RawWaker, RawWakerVTable, Waker}};

use alloc::sync::Arc;

use crate::task::add_task;

use super::{schedule::Task,   TaskStatus};

const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, drop);

/// 直接根据 Task 的指针重新构造 Waker
unsafe fn clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &VTABLE)
}

/// 根据 Waker 内部的无类型指针，得到 Task 的指针，唤醒任务
unsafe fn wake(p: *const ()) {
    wakeup_task(p as *const Task)
}

/// 创建 waker 时没有增加引用计数，因此不需要实现 Drop
unsafe fn drop(_p: *const ()) {}

/// 只有在运行的任务才需要 waker，
/// 只需要从 CurrentTask 中获取任务的原始指针
pub fn waker_from_task(task_ptr: *const Task) -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(task_ptr as _, &VTABLE)) }
}
// 这里不对任务的状态进行修改，在调用 waker.wake() 之前对任务状态进行修改
/// 这里直接使用 Arc，会存在问题，导致任务的引用计数减一，从而直接被释放掉
/// 因此使用任务的原始指针，只在确实需要唤醒时，才会拿到任务的 Arc 指针
pub fn wakeup_task(task_ptr: *const Task) {
    let task = unsafe { &*task_ptr };
    let mut state=task.state_lock_manual();
    match **state {
        // 任务正在运行，且没有让权，不必唤醒
        // 可能不止一个其他的任务在唤醒这个任务，因此被唤醒的任务可能是处于 Running 状态的
        TaskStatus::Running => (),
        // 任务准备让权，但没有让权，还在核上运行，但已经被其他核唤醒，此时只需要修改其状态即可
        // 后续的处理由正在核上运行的自己来决定
        TaskStatus::Blocking => **state = TaskStatus::Waked,
        // 任务不在运行，但其状态处于就绪状态，意味着任务已经在就绪队列中，不需要再向其中添加任务
        TaskStatus::Runnable => (),
        // 任务不在运行，已经让权结束，不在核上运行，就绪队列中也不存在，需要唤醒
        // 只有处于 Blocked 状态的任务才能被唤醒，这时候才会拿到任务的 Arc 指针
        TaskStatus::Blocked => {
            **state = TaskStatus::Runnable;
            let task_ref = unsafe { Arc::from_raw(task_ptr) };

            info!("task wakeup   tid:{}",task_ref.id());
             add_task(task_ref);

        }
        TaskStatus::Waked => panic!("cannot wakeup Waked "),
        // 无法唤醒已经退出的任务
        TaskStatus::Zombie=> panic!("cannot wakeup Exited "),
    };
    core::mem::drop(core::mem::ManuallyDrop::into_inner(state));

}

/// 自定义一个 noop waker：  
/// 它的 wake/wake_by_ref/drop 都是空操作，因为我们保证 future 立刻 ready。
pub fn custom_noop_waker() -> Waker {
    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(core::ptr::null(), &VTABLE), // clone
        |_| {},                                       // wake
        |_| {},                                       // wake_by_ref
        |_| {},                                       // drop
    );
    let raw = RawWaker::new(core::ptr::null(), &VTABLE);
    // Safety: VTABLE 是有效的，所有操作都无副作用，waker 的生命周期由 Context 管理
    unsafe { Waker::from_raw(raw) }
}