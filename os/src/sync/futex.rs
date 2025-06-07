use core::{future::Future, pin::Pin, task::{Context, Poll, Waker}};

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};
use lazy_init::LazyInit;
use spin::mutex::Mutex as SpinMutex;

use crate::{mm::{get_target_ref, VirtAddr}, sync::waitqueue::{GeneralWaitTaskList, GeneralWaitWakerNode}, task::{current_task_id, sleeplist::SleepFuture, Task}, timer::TimeVal, utils::error::SysErrNo};


pub type FutexKey = (usize, usize); // (process_token, user_virtual_address)
pub static GLOBAL_FUTEX_SYSTEM: LazyInit<SpinMutex<BTreeMap<FutexKey, FutexWaitQueue>>> =
    LazyInit::new();
pub fn init_futex_system() {
    GLOBAL_FUTEX_SYSTEM.init_by(SpinMutex::new(BTreeMap::new()));
}
struct FutexWaiter {
    waker: Waker,       // 等待任务的 Waker
    task_id: usize,     // (可选) 用于调试或在复杂场景下精确移除
    // Arc<Task> 也可以，但只存 Waker 更轻量，Waker 已包含唤醒任务所需信息
}


#[derive(Default)]
pub struct FutexWaitQueue {
    // 内部直接使用 GeneralWaitTaskList 来管理 Waker
    task_list: GeneralWaitTaskList,
}

impl FutexWaitQueue {
    pub fn new() -> Self {
        Default::default()
    }

    /// 将等待任务的 Waker (和 task_id) 添加到队列。
    pub fn add_waiter(&mut self, node: Arc<GeneralWaitWakerNode>) {
        self.task_list.add_waiter(node);
    }
   
    /// 唤醒队列中的一个任务。
    /// 返回被唤醒的 Waker (Option<Waker>)，如果需要的话，但通常直接调用 wake()。
    /// 这里我们遵循 GeneralWaitTaskList::notify_one 的行为，它返回 bool。
    pub fn wake_one(&mut self) -> bool {
        self.task_list.notify_one()
    }

    /// 唤醒最多 num 个任务。
    /// 返回实际唤醒的 Waker 列表 (Vec<Waker>) 或实际唤醒的数量。
    /// 这里我们让它返回 Vec<Waker> 以便 sys_futex 可以用其长度。
    pub fn wake_num(&mut self, num_to_wake: usize) -> usize {
        self.task_list.notify_n(num_to_wake)
    }

    pub fn is_empty(&self) -> bool {
        self.task_list.is_empty()
    }

    
    /// 移除与特定 Arc<GeneralWaitWakerNode> 关联的 Waker。
    /// FutexWaitInternalFuture 需要持有这个 Arc。
    pub fn remove_specific_waiter(&mut self, node_arc: &Arc<GeneralWaitWakerNode>) {
        self.task_list.remove_waiter(node_arc);
    }
}


pub struct FutexWaitInternalFuture {
    token: usize,
    uaddr_va: usize,
    expected_val: u32,
    timeout_sleep: Option<SleepFuture>,
    futex_key: FutexKey,
    // 存储已注册到 futex 等待队列的节点 Arc，用于精确移除
    registered_futex_waiter_node: Option<Arc<GeneralWaitWakerNode>>,
}

impl FutexWaitInternalFuture {
    pub fn new(
        token: usize,
        uaddr_va: usize,
        expected_val: u32,
        deadline: Option<TimeVal>,
    ) -> Self {
        Self {
            token,
            uaddr_va,
            expected_val,
            timeout_sleep: deadline.map(|d| crate::task::sleeplist::sleep_until(Some(d))), // 使用你的 sleep_until
            futex_key: (token, uaddr_va),
            registered_futex_waiter_node: None,
        }
    }
}

impl Future for FutexWaitInternalFuture {
    type Output = Result<(), SysErrNo>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        // 1. 再次检查用户空间的 futex 值
        let current_val_in_user =*match get_target_ref(this.token, this.uaddr_va as *const u32){
            Ok(v) => v,
            Err(e) => {
                if let Some(node_arc) = this.registered_futex_waiter_node.take() {
                    GLOBAL_FUTEX_SYSTEM.lock().get_mut(&this.futex_key).map(|wq| wq.remove_specific_waiter(&node_arc));
                }
                return Poll::Ready(Err(e.into()));
            }
        };
       
        if current_val_in_user != this.expected_val {
            if let Some(node_arc) = this.registered_futex_waiter_node.take() {
                GLOBAL_FUTEX_SYSTEM.lock().get_mut(&this.futex_key).map(|wq| wq.remove_specific_waiter(&node_arc));
            }
            return Poll::Ready(Err(SysErrNo::EAGAIN));
        }

        // 2. 检查超时
        if let Some(sleep_future) = &mut this.timeout_sleep {
            let mut pinned_sleep = unsafe { Pin::new_unchecked(sleep_future) };
            if let Poll::Ready(()) = pinned_sleep.as_mut().poll(cx) {
                if let Some(node_arc) = this.registered_futex_waiter_node.take() {
                    GLOBAL_FUTEX_SYSTEM.lock().get_mut(&this.futex_key).map(|wq| wq.remove_specific_waiter(&node_arc));
                }
                return Poll::Ready(Err(SysErrNo::ETIMEDOUT));
            }
        }

        // 3. 如果 Waker 尚未注册到 futex 队列，则注册
        if this.registered_futex_waiter_node.is_none() {
            let mut futex_system_guard = GLOBAL_FUTEX_SYSTEM.lock();
            let wait_queue = futex_system_guard
                .entry(this.futex_key)
                .or_insert_with(FutexWaitQueue::new);

            let waiter_node = Arc::new(GeneralWaitWakerNode::new(cx.waker().clone()));
            wait_queue.add_waiter(waiter_node.clone()); // add_waiter 现在接收 Arc<GeneralWaitWakerNode>
            this.registered_futex_waiter_node = Some(waiter_node);
            let task = cx.waker().data() as *const Task;
            unsafe { &*task }.set_state(crate::task::TaskStatus::Blocking);
 
            
        }
        Poll::Pending
    }
}

impl Drop for FutexWaitInternalFuture {
    fn drop(&mut self) {
        if let Some(node_arc) = self.registered_futex_waiter_node.take() {
            log::trace!("FutexWaitInternalFuture dropped for key {:?} removing waiter.", self.futex_key);
            let mut futex_system_guard = GLOBAL_FUTEX_SYSTEM.lock();
            if let Some(wq) = futex_system_guard.get_mut(&self.futex_key) {
                wq.remove_specific_waiter(&node_arc); // 使用 Arc::ptr_eq 来精确移除
                if wq.is_empty() {
                    futex_system_guard.remove(&self.futex_key);
                }
            }
        }
    }
}
