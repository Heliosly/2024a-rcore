
use alloc::sync::Arc;
use alloc::collections::LinkedList;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use spin::mutex::Mutex as SpinMutex;

use crate::task::{waker, Task, TaskStatus}; // 假设使用的是 spin crate 的 Mutex

// --- GeneralWaitWakerNode ---

/// 代表等待队列中的一个节点，持有特定任务的 `Waker`。
///
/// 当一个任务需要等待时，它会创建一个 `GeneralWaitWakerNode` 并附带其 waker。
/// 这个节点随后被添加到 `GeneralWaitTaskList` 中。当任务等待的条件满足时，
/// 此节点中的 waker 会被用来通知任务的执行器，该任务可以再次被轮询。
pub struct GeneralWaitWakerNode {
    /// 与等待任务相关联的 waker。
    waker: Waker,
}

impl GeneralWaitWakerNode {
    /// 创建一个新的 `GeneralWaitWakerNode`。
    ///
    /// # Arguments
    ///
    /// * `waker`: 将要等待的任务的 `Waker`。
    pub fn new(waker: Waker) -> Self {
        Self { waker }
    }
}

// --- GeneralWaitTaskList ---

/// 一个等待特定条件的任务列表（由 `GeneralWaitWakerNode` 代表）。
///
/// 这个列表通常在像 `WaitQueue` 这样的同步原语内部管理。
/// 它使用 `LinkedList` 来允许高效地添加和移除等待者，
/// 特别是对于 `notify_one`（唤醒最旧的等待者）操作。
/// `Arc` 用于允许对 `GeneralWaitWakerNode` 的共享所有权，
/// 这对于移除操作以及让 future 持有对其节点的引用是必需的。
#[derive(Default)]
pub struct GeneralWaitTaskList {
    /// 底层的 waker 节点列表。
    list: LinkedList<Arc<GeneralWaitWakerNode>>,
}

impl GeneralWaitTaskList {
    pub fn is_empty(&self)->bool{
        self.list.is_empty()
    }
    /// 创建一个新的、空的 `GeneralWaitTaskList`。
    pub fn new() -> Self {
        Default::default()
    }

    /// 将一个等待者的节点添加到列表的末尾。
    ///
    /// # Arguments
    ///
    /// * `node_arc`: 一个指向要添加的 `GeneralWaitWakerNode` 的 `Arc`。
    ///               `Arc` 允许节点在列表和拥有 waker 的 future 之间共享。
    pub fn add_waiter(&mut self, node_arc: Arc<GeneralWaitWakerNode>) {
        self.list.push_back(node_arc);
    }

    /// 从列表中移除一个特定的等待者节点。
    ///
    /// 此方法遍历列表，通过比较原始指针（使用 `Arc::ptr_eq`）来查找与 `node_to_remove_arc` 匹配的节点。
    /// 如果找到，它会移除该节点并返回它。否则，返回 `None`。
    ///
    /// # Arguments
    ///
    /// * `node_to_remove_arc`: 一个引用要移除的节点的 `Arc`。
    ///
    /// # Returns
    ///
    /// * `Some(Arc<GeneralWaitWakerNode>)` 如果节点被找到并移除。
    /// * `None` 如果在列表中未找到该节点。
    pub fn remove_waiter(&mut self, node_to_remove_arc: &Arc<GeneralWaitWakerNode>) -> Option<Arc<GeneralWaitWakerNode>> {
        let mut current_idx = 0;
        let mut found_idx = None;

        // 遍历以查找要移除节点的索引。
        // 我们使用 Arc::ptr_eq 进行直接指针比较，这在这里是高效且正确的，
        // 因为我们正在寻找完全相同的 Arc 实例。
        for node_in_list_arc in self.list.iter() {
            if Arc::ptr_eq(node_in_list_arc, node_to_remove_arc) {
                found_idx = Some(current_idx);
                break;
            }
            current_idx += 1;
        }

        if let Some(idx_to_remove) = found_idx {
            // `split_off` 在 `idx_to_remove` 处分割列表。
            // `idx_to_remove` 处的元素成为新列表 (`tail`) 的第一个元素。
            // `self.list` 现在包含从 `0` 到 `idx_to_remove - 1` 的元素。
            let mut tail = self.list.split_off(idx_to_remove);

            // `pop_front` 从 `tail` 中移除并返回第一个元素，即我们的目标节点。
            let removed_node = tail.pop_front();

            // 将 `tail` 中剩余的元素（如果有的话）追加回 `self.list`。
            // 这有效地将 `idx_to_remove` 之后的元素重新连接起来。
            self.list.append(&mut tail);
            removed_node
        } else {
            // 如果没有找到匹配的节点。
            None
        }
    }

    /// 通知（唤醒）列表中的一个等待者。
    ///
    /// 它会移除列表头部的节点（最早的等待者）并调用其 waker。
    ///
    /// # Returns
    ///
    /// * `true` 如果成功唤醒了一个等待者。
    /// * `false` 如果列表为空，没有等待者被唤醒。
    pub fn notify_one(&mut self) -> bool {
        if let Some(node_arc) = self.list.pop_front() {
            // 使用 wake_by_ref 来唤醒任务，而不需要消耗 Waker 本身。
            // 这允许 Waker 在未来可能被再次使用（尽管在这个特定场景下，
            // 一旦被唤醒，节点通常会被移除或 Future 会完成）。
            node_arc.waker.wake_by_ref();
            true
        } else {
            false
        }
    }
    pub fn notify_n(&mut self, num_to_wake: usize) -> usize { // 返回唤醒的数量
        let mut woken_count = 0;
        for _ in 0..num_to_wake {
            if let Some(node_arc) = self.list.pop_front() {
                node_arc.waker.wake_by_ref();
                woken_count += 1;
            } else {
                break; // 队列已空
            }
        }
        woken_count
    }
}

// --- WaitQueue ---

/// 等待队列。
///
/// 允许多个任务等待某个条件变为真。当条件可能已改变时，
/// 可以通过 `notify_one` 或类似的机制来唤醒一个等待的任务。
///
/// 它内部使用 `SpinMutex` 来保护对 `GeneralWaitTaskList` 的并发访问。
#[derive(Default)]
pub struct WaitQueue {
    /// 受 `SpinMutex` 保护的等待任务列表。
    queue: SpinMutex<GeneralWaitTaskList>,
}

impl WaitQueue {
    /// 创建一个新的 `WaitQueue`。
    pub fn new() -> Self {
        Self {
            queue: SpinMutex::new(GeneralWaitTaskList::new()),
        }
    }

    /// 通知（唤醒）队列中的一个等待者。
    ///
    /// 获取锁，然后从内部的 `GeneralWaitTaskList` 中唤醒一个任务。
    ///
    /// # Returns
    ///
    /// * `true` 如果成功唤醒了一个等待者。
    /// * `false` 如果队列为空。
    pub fn notify_one(&self) -> bool {
        self.queue.lock().notify_one()
    }

    /// 创建一个 Future，该 Future 会一直等待直到提供的 `condition` 函数返回 `true`。
    ///
    /// # Arguments
    ///
    /// * `condition`: 一个闭包，它会被重复调用以检查等待的条件是否已满足。
    ///   - `Fn() -> bool`: 闭包不接受参数并返回一个布尔值。
    ///   - `Unpin`: 闭包必须是 `Unpin` 的，因为它会被存储在 `WaitUntilFuture` 中。
    ///   - `Send + Sync + 'a`: 如果 `WaitQueue` 可能在线程间共享并且 Future 可能会被发送到其他线程，
    ///     那么闭包也需要是 `Send` 和 `Sync` 的。`'a` 生命周期确保闭包的生命周期
    ///     至少与 `WaitQueue` 的引用一样长。
    ///
    /// # Returns
    ///
    /// 一个 `WaitUntilFuture` 实例，可以被 `.await`。
    pub fn wait_until<'a, F>(&'a self, condition: F) -> WaitUntilFuture<'a, F>
    where
        F: Fn() -> bool + Unpin + Send + Sync + 'a, // 为 F 添加 Send + Sync 约束
    {
        WaitUntilFuture {
            wq: self,
            condition,
            registered_node_arc: None,
        }
    }
}

// --- WaitUntilFuture ---

/// 一个 Future，它会异步地等待某个条件满足。
///
/// 这个 Future 在被轮询时：
/// 1. 检查条件。如果满足，Future 完成。
/// 2. 如果条件不满足且尚未注册，它会将当前任务的 waker 注册到 `WaitQueue` 中。
/// 3. 返回 `Poll::Pending`，直到被 `WaitQueue` 唤醒。
///
/// `#[must_use]` 属性提示用户这个 Future 必须被轮询（例如通过 `.await`）才能执行任何操作。
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WaitUntilFuture<'a, F: Fn() -> bool + Unpin> {
    /// 对 `WaitQueue` 的引用，用于注册和注销 waker。
    wq: &'a WaitQueue,
    /// 用于检查等待条件的函数。
    condition: F,
    /// 如果 Future 当前正在等待，这里会存储一个指向其在 `WaitQueue` 中注册节点的 `Arc`。
    /// `None` 表示 Future 尚未注册，或者条件已满足且节点已被移除。
    registered_node_arc: Option<Arc<GeneralWaitWakerNode>>,
}

impl<'a, F: Fn() -> bool + Unpin> Future for WaitUntilFuture<'a, F> {
    /// 此 Future 完成时的输出类型。这里是 `()` 因为它仅用于同步。
    type Output = ();

    /// 轮询此 Future 的状态。
    ///
    /// # Arguments
    ///
    /// * `self`: 对 Future 的 `Pin<&mut Self>` 引用。
    /// * `cx`: 当前任务的上下文，包含用于唤醒任务的 `Waker`。
    ///
    /// # Returns
    ///
    /// * `Poll::Ready(())`：如果 `condition` 返回 `true`。
    /// * `Poll::Pending`：如果 `condition` 返回 `false`。在这种情况下，如果尚未注册，
    ///   Future 会将当前任务的 waker 注册到 `WaitQueue`。
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 安全：我们不会移动 `self` 内部的数据，`get_mut` 提供了对固定数据的可变引用。
        let mut_self = self.get_mut();

        // 1. 检查条件是否已满足。
        if (mut_self.condition)() {
            // 条件满足。
            // 如果我们之前注册了一个 waker 节点，现在需要从等待队列中移除它。
            if let Some(node_arc) = mut_self.registered_node_arc.take() {
                // 获取锁并尝试移除节点。
                // 此时，即使 remove_waiter 失败（例如，由于竞争条件导致节点已被唤醒并移除），
                // 也是安全的，因为我们的目标是确保它最终被移除。
                mut_self.wq.queue.lock().remove_waiter(&node_arc);
            }
            // Future 完成。
            return Poll::Ready(());
        }

        // 2. 条件不满足。检查我们是否已经注册到等待队列。
        if mut_self.registered_node_arc.is_none() {
            // 尚未注册，需要将当前任务的 waker 添加到等待队列。
            // 创建一个新的 waker 节点。
            let task = cx.waker().data() as *const Task;
            unsafe { &*task }.set_state(TaskStatus::Blocking);
            info!("blocking in wait_until");
            let node = GeneralWaitWakerNode::new(cx.waker().clone());
            let node_arc = Arc::new(node);
          
            // 获取锁并将节点添加到队列。
            mut_self.wq.queue.lock().add_waiter(node_arc.clone());

            // 保存对节点的 Arc 引用，以便在 Future 完成或被丢弃时可以移除它。
            mut_self.registered_node_arc = Some(node_arc);
        }
       

        trace!("WaitUntilFutex Pending in ?");
        Poll::Pending
    }
}

impl<'a, F: Fn() -> bool + Unpin> Drop for WaitUntilFuture<'a, F> {
    /// 当 `WaitUntilFuture` 被丢弃时执行清理。
    ///
    /// 如果 Future 在完成前被丢弃（例如，任务被取消），
    /// 确保从 `WaitQueue` 中移除其注册的 waker 节点，以防止悬挂的 waker
    /// 和内存泄漏。
    fn drop(&mut self) {
        if let Some(node_arc) = self.registered_node_arc.take() {
            // 获取锁并尝试移除节点。
            // `take()` 会将 `registered_node_arc` 置为 `None`，防止双重移除。
            self.wq.queue.lock().remove_waiter(&node_arc);
            // 注意：如果此时 `remove_waiter` 失败（例如，节点已被 notify_one 移除），
            // 这是可以接受的，因为我们的目标是确保它不留在队列中。
        }
    }
}