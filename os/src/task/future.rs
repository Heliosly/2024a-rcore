use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use alloc::{boxed::Box, collections::vec_deque::VecDeque, vec::Vec};


#[derive(Debug)]
pub struct YieldFuture {
    _has_polled: bool,
  
}

impl YieldFuture {
    pub fn new() -> Self {
        // 这里获取中断状态，并且关中断
     
        Self {
            _has_polled: false,
            
        }
    }
}
///yieldnow
pub fn  yield_now() -> YieldFuture {
    YieldFuture::new()
}
impl Future for YieldFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            let this = self.get_mut();
            if this._has_polled {
                // 恢复原来的中断状态
               
                Poll::Ready(())
            } else {
                // trace!("yield_now");
                //第一次执行到这
                this._has_polled = true;
                Poll::Pending
            }
        }
    }
}





use crate::task::{TaskRef, TaskStatus}; // 或你内核中的其他 Mutex


pub struct JoinFuture {
    target_task: TaskRef, // 要等待的目标任务
}

impl JoinFuture {
    pub fn new(target_task: TaskRef) -> Self {
        Self { target_task }
    }
}

impl Future for JoinFuture {
    type Output = isize; // 返回目标任务的退出码

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `self` 是 Pin<&mut JoinFuture>
        // `this` 是 &mut JoinFuture
        let this = self.get_mut(); // 如果 JoinFuture 是 Unpin

        // 1. 获取目标任务状态和退出码。这需要锁住目标任务的内部状态。
        //    这假设 Task 结构体有一个内部 Mutex 保护其 status 和 exit_code。
        //    let mut target_task_inner_guard = this.target_task.inner.lock(); // 示例锁
        //
        //    或者，Task 提供原子或锁保护的访问器：
        let target_status = this
        .target_task
        .state.lock(); 

        if *target_status == TaskStatus::Zombie{
            // 目标任务已经退出
            let exit_code = this.target_task.get_exit_code(); // 假设这个方法内部处理了锁
            // log::trace!("JoinFuture: Target task {} already exited with code {:?}.",
            //             this.target_task.id(), exit_code);
            Poll::Ready(exit_code)
        } else {
            // 目标任务尚未退出，注册当前 Future 的 Waker 到目标任务的 join_waiters 列表
            // log::trace!("JoinFuture: Target task {} not exited. Registering waker.",
            //             this.target_task.id());
            drop(target_status);
            this.target_task.join(cx.waker().clone());
            // 释放目标任务的锁（如果之前获取了）
            // drop(target_task_inner_guard);

            // 再次检查状态，以防在注册 Waker 和释放锁之间任务恰好退出了 (经典的 check-then-act 问题)
            // 这是为什么需要在 Task::register_join_waker 内部或 Task::notify_join_waiters_and_exit
            // 内部进行原子性的状态检查和 Waker 操作。
            // 更好的做法是：Task::register_join_waker 如果发现任务已退出，则直接返回一个标志。
            //
            // 简化假设：如果 register_join_waker 之后任务退出，它会唤醒我们。
            // 如果在 register_join_waker 之前任务退出，上面的分支会处理。
            // 如果在 register_join_waker 过程中任务退出（在锁的保护下），
            //   那么 register_join_waker 应该能安全处理，或者 notify_join_waiters
            //   应该能看到新注册的 waker。

            // 再次检查，以防万一 (可选的，取决于 register_join_waker 的原子性)
            let new_target_status = this.target_task.state.lock();
            if *new_target_status == TaskStatus::Zombie{
                // 在我们注册Waker后，但在返回Pending前，目标任务退出了。
                // Waker 可能已经被调用了，也可能没有（取决于精确的时序和锁的粒度）。
                // 为了确保我们得到结果，再次获取退出码。
                log::trace!("JoinFuture: Target task {} exited race condition after waker registration.",
                            this.target_task.id());
                Poll::Ready(this.target_task.get_exit_code())
            } else {
                trace!("JoinFuture Pending In Wait");
                Poll::Pending // 等待目标任务退出并唤醒我们
            }
        }
    }
}

/// WaitAnyFuture：等待一组 JoinFuture 中的任意一个完成
pub struct WaitAnyFuture {
    // 存储 (pid, JoinFuture)
    futures: VecDeque<(usize, Pin<Box<JoinFuture>>)>,
}

impl WaitAnyFuture {
    pub fn new(tasks: Vec<TaskRef>) -> Self {
        let futures = tasks
            .into_iter()
            .map(|t| {
                let pid = t.get_pid();
                (pid, Box::pin(JoinFuture::new(t)))
            })
            .collect::<VecDeque<_>>();

        WaitAnyFuture { futures }
    }
}

impl Future for WaitAnyFuture {
    type Output = usize; // 返回第一个完成任务的 pid

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut pending = VecDeque::new();

        while let Some((pid, mut join_fut)) = this.futures.pop_front() {
            match join_fut.as_mut().poll(cx) {
                Poll::Ready(_) => {
                    // 只要有一个任务完成，就直接返回它的 pid
                    return Poll::Ready(pid);
                }
                Poll::Pending => {
                    pending.push_back((pid, join_fut));
                }
            }
        }

        this.futures = pending;
        trace!("WaitAnyFuture  Pending  in init");
        Poll::Pending
    }
}