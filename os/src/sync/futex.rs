use alloc::collections::{BTreeMap, LinkedList}; // LinkedList for FutexWaitQueue
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use core::task::{Context, Poll, Waker};

use lazy_init::LazyInit;
use spin::mutex::Mutex as SpinMutex;

use crate::mm::get_target_ref;
use crate::task::waker::waker_from_task;
// 你的模块依赖
use crate::task::{current_task_id, TaskStatus, TID2TC};
use crate::timer::{current_time, TimeVal, UserTimeSpec};

use crate::task::sleeplist::{sleep_until, SleepFuture};
use crate::utils::error::SysErrNo;

// --- FutexWaiterNode (专为 Futex 设计) ---
#[derive(Debug)] // Debug for logging
pub struct FutexWaiterNode {
    pub waker: Waker,      // 存储任务的 Waker
    pub task_id: usize,    // 任务 ID
    pub wait_bitmask: u32, // 等待的位掩码 (FUTEX_WAIT_BITSET)
}

impl FutexWaiterNode {
    pub fn new(waker: Waker, task_id: usize, wait_bitmask: u32) -> Self {
        Self {
            waker,
            task_id,
            wait_bitmask,
        }
    }
}
// 当 Arc<FutexWaiterNode> 被 drop 时，FutexWaiterNode 被 drop，其 Waker 成员被 drop。
// Waker 的 drop 会调用 VTABLE 中的 drop_raw。

// --- FutexWaitQueue (使用 LinkedList<Arc<FutexWaiterNode>>) ---
#[derive(Default, Debug)]
pub struct FutexWaitQueue {
    waiters: LinkedList<Arc<FutexWaiterNode>>,
}

impl FutexWaitQueue {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn add_waiter(&mut self, node_arc: Arc<FutexWaiterNode>) {
        // 可选：基于 task_id 或 Waker::will_wake 进行去重/更新
        // 为简单和高效的 FIFO，先直接 push_back
        self.waiters.push_back(node_arc);
    }

    /// 移除特定的 Arc<FutexWaiterNode> 实例 (用于 FutexWaitInternalFuture::drop)
    pub fn remove_specific_waiter(&mut self, node_to_remove_arc: &Arc<FutexWaiterNode>) {
        // LinkedList 没有高效的按值移除，需要迭代
        // 但我们可以通过 Arc::ptr_eq 来精确匹配
        self.waiters
            .retain(|node_in_list| !Arc::ptr_eq(node_in_list, node_to_remove_arc));
    }

    /// 唤醒 num_to_wake 个匹配 bitmask 的等待者。
    /// 返回实际唤醒的数量。
    pub fn wake_matching_waiters(&mut self, num_to_wake: usize, wake_bitmask: u32) -> usize {
        let mut woken_count = 0;
        let mut not_woken_yet = LinkedList::new(); // 临时存储未被唤醒的

        while let Some(node_arc) = self.waiters.pop_front() {
            if woken_count < num_to_wake && (node_arc.wait_bitmask & wake_bitmask) != 0 {
                node_arc.waker.wake_by_ref();
                woken_count += 1;
            } else {
                not_woken_yet.push_back(node_arc); // 未唤醒，放回
            }
        }
        // 将未唤醒的节点重新加回主列表 (保持顺序)
        self.waiters.append(&mut not_woken_yet);
        woken_count
    }

    pub fn is_empty(&self) -> bool {
        self.waiters.is_empty()
    }
}

// --- FutexKey 和 GLOBAL_FUTEX_SYSTEM (同前) ---
pub type FutexKey = (usize, usize); // (process_token, user_virtual_address)
pub static GLOBAL_FUTEX_SYSTEM: LazyInit<SpinMutex<BTreeMap<FutexKey, FutexWaitQueue>>> =
    LazyInit::new();
pub fn init_futex_system() {
    GLOBAL_FUTEX_SYSTEM.init_by(SpinMutex::new(BTreeMap::new()));
}

// --- FutexWaitInternalFuture (管理异步等待逻辑) ---
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct FutexWaitInternalFuture {
    token: usize,
    uaddr_va: usize,
    expected_val: u32,
    wait_bitmask: u32,
    timeout_sleep: Option<SleepFuture>, // 用于超时
    futex_key: FutexKey,
    // 存储已注册到 futex 等待队列的节点 Arc，用于在 Drop 时精确移除
    registered_node_arc: Option<Arc<FutexWaiterNode>>,
}

impl FutexWaitInternalFuture {
    pub fn new(
        token: usize,
        uaddr_va: usize,
        expected_val: u32,
        wait_bitmask: u32,
        deadline: Option<TimeVal>,
    ) -> Self {
        Self {
            token,
            uaddr_va,
            expected_val,
            wait_bitmask,
            timeout_sleep: deadline.map(|d| sleep_until(Some(d))),
            futex_key: (token, uaddr_va),
            registered_node_arc: None,
        }
    }
}
impl Future for FutexWaitInternalFuture {
    type Output = Result<(), SysErrNo>; // Ok(()) 表示被成功唤醒或条件不再满足, Err 表示超时或错误

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        info!("poll futex future");
        let this = self.as_mut().get_mut();
        // 1. 检查用户空间的 futex 值 (只在首次尝试注册 Waker 前检查)
        let current_val_in_user =
            *match unsafe { get_target_ref(this.token, this.uaddr_va as *const u32) } {
                Ok(v) => v,
                Err(e) => {
                    // this.cleanup_registration_from_futex_queue(); // 此时尚未注册
                    return Poll::Ready(Err(e.into())); // 读取用户内存失败
                }
            };

        info!("poll futex future1");
        // 检查是否之前已经注册过 Waker 并且现在被 poll
        // 这通常意味着是被 FUTEX_WAKE 唤醒了 (或者是一个超时唤醒后，SleepFuture 的 Ready 导致这里重 poll,或者是被信号打断)
        if current_val_in_user != this.expected_val {
            if this.registered_node_arc.is_some() {
                // 如果是被唤醒，我们不再重新检查用户空间的 futex 值。
                // 假设唤醒是有效的，FutexWaitInternalFuture 完成。
                // 用户空间代码在 sys_futex 返回后会自己重新检查和竞争锁。
                // 清理在全局队列中的注册。
                info!("tid{}  futex is finish",current_task_id());
                this.cleanup_registration_from_futex_queue();
                return Poll::Ready(Ok(())); // <--- 被唤醒，返回成功
            }
        }

        info!("poll futex future2");
        if current_val_in_user != this.expected_val {
            // this.cleanup_registration_from_futex_queue(); // 此时尚未注册
            return Poll::Ready(Err(SysErrNo::EAGAIN)); // 值不匹配，不等待
        }

        // 2. 检查超时 (如果设置了)
        if let Some(sleep_future) = &mut this.timeout_sleep {
            let mut pinned_sleep = unsafe { Pin::new_unchecked(sleep_future) };
            if let Poll::Ready(()) = pinned_sleep.as_mut().poll(cx) {
                // 超时发生。注意：此时 Waker 可能还未在 futex 队列注册，
                // 或者即使注册了，超时优先。
                // cleanup_registration_from_futex_queue 会处理 registered_node_arc 是 None 的情况。
                this.cleanup_registration_from_futex_queue();
                info!("poll futex is timeout");
                return Poll::Ready(Err(SysErrNo::ETIMEDOUT));
            }
            // 如果 sleep_future 返回 Pending，cx.waker() 已被 SleeperList 注册
            // 这意味着 cx.waker() (即 FutexWaitInternalFuture 的 Waker)
            // 既可能被超时唤醒，也可能被 FUTEX_WAKE 唤醒。
            // 我们上面的第一个 if this.registered_node_arc.is_some() 会处理被WAKE的情况。
        }

        // 3. 如果执行到这里，意味着：
        //    a. Futex 值与期望值匹配。
        //    b. 未超时 (或者超时 Future 返回了 Pending)。

        info!("poll futex future3");
        if this.registered_node_arc.is_none() {
            //说明第一次poll
            let task_arc_for_waker = match TID2TC.lock().get(&current_task_id()).cloned() {
                Some(arc) => arc,
                None => return Poll::Ready(Err(SysErrNo::ESRCH)),
            };
            let task_ptr = Arc::into_raw(task_arc_for_waker);
            let waker_instance = unsafe { waker_from_task(task_ptr) };
            let waiter_node = Arc::new(FutexWaiterNode::new(
                waker_instance,
                current_task_id(),
                this.wait_bitmask,
            ));

            let mut futex_system_guard = GLOBAL_FUTEX_SYSTEM.lock();
            let wait_queue = futex_system_guard
                .entry(this.futex_key)
                .or_insert_with(FutexWaitQueue::new);
            wait_queue.add_waiter(waiter_node.clone()); // add_waiter 现在是 FutexWaitQueue 的方法
            drop(futex_system_guard);

            this.registered_node_arc = Some(waiter_node);

            // 设置任务状态为 Blocked
            let task_ref_for_status_change = unsafe { Arc::from_raw(task_ptr) };
            let mut state_guard = task_ref_for_status_change.state_lock_manual();
            **state_guard=TaskStatus::Blocking;
            drop(core::mem::ManuallyDrop::into_inner(state_guard))
            ;
            drop(task_ref_for_status_change);
            info!("Futex Pending in init");
            Poll::Pending // 等待被 FUTEX_WAKE 或超时唤醒
        } else {
            //说明被中断
            
        info!("poll futex is EINTR");
            // bpoint();
            return Poll::Ready(Err(SysErrNo::EINTR));
        }
    }
}

impl FutexWaitInternalFuture {
    fn cleanup_registration_from_futex_queue(&mut self) {
        if let Some(node_arc) = self.registered_node_arc.take() {
            // node_arc 是 Arc<FutexWaiterNode>
            let mut futex_system_guard = GLOBAL_FUTEX_SYSTEM.lock();
            if let Some(wq) = futex_system_guard.get_mut(&self.futex_key) {
                wq.remove_specific_waiter(&node_arc); // 使用 Arc::ptr_eq 移除
                if wq.is_empty() {
                    futex_system_guard.remove(&self.futex_key);
                }
            }
            // 当 node_arc (最后一个Arc引用) 在这里或外层被drop时, FutexWaiterNode 会被drop,
            // 其内部的 Waker 会被drop, 触发 Waker VTABLE 的 drop_raw,
            // drop_raw 应该 Arc::from_raw(task_ptr) 来释放 Task 的 Arc。
        }
    }
}

impl Drop for FutexWaitInternalFuture {
    fn drop(&mut self) {
        self.cleanup_registration_from_futex_queue();
        // SleepFuture 的 Drop 会自动处理其在 SleeperList 中的注册
    }
}
