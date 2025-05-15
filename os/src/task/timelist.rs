use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::collections::LinkedList; 
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::cmp::Ordering;

use spin::mutex::Mutex;

// 从你提供的上下文引入
use crate::timer::{TimeVal, current_time};
use crate::task::{Task, TaskStatus, add_task};
use crate::task::waker::waker_from_task;

// 全局的 CURRENT_TICK，由中断处理程序更新
// static CURRENT_TICK: AtomicU64 = AtomicU64::new(0);

fn current_running_task_ptr() -> *const Task {
    // FIXME: 实现这个关键函数
    unsafe { static mut FAKE_TASK_FOR_WAKER: Option<Arc<Task>> = None;
        if FAKE_TASK_FOR_WAKER.is_none() {
            let arc_task = Arc::new(Task::new_kernel_task_empty_id(999_usize));
            FAKE_TASK_FOR_WAKER = Some(arc_task);
        }
        Arc::as_ptr(FAKE_TASK_FOR_WAKER.as_ref().unwrap())
    }
}


/// 1. 睡眠节点：存储唤醒时间和任务的 Waker 及 Task 指针
///    (不再需要 intrusive-collections 的 LinkedListLink)
pub struct SleepNode {
    deadline: TimeVal,
    waker: Waker,
    task_ptr: *const Task, // 用于调试、排序稳定性、或潜在的非 Arc 比较
}

impl SleepNode {
    pub fn new(deadline: TimeVal, waker: Waker, task_ptr: *const Task) -> Self {
        Self {
            deadline,
            waker,
            task_ptr,
        }
    }
}

// SleepNode 的比较逻辑主要基于 deadline，然后是 task_ptr
impl PartialEq for SleepNode {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.task_ptr == other.task_ptr
    }
}
impl Eq for SleepNode {}

impl PartialOrd for SleepNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SleepNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deadline.cmp(&other.deadline)
            .then_with(|| self.task_ptr.cmp(&other.task_ptr))
    }
}


/// 2. 睡眠者列表：使用 alloc::collections::LinkedList 存储 Arc<SleepNode>
pub struct SleeperList {
    // 存储 Arc<SleepNode>，并保持按 deadline 有序
    list: LinkedList<Arc<SleepNode>>,
}

impl SleeperList {
    pub fn new() -> Self {
        Self { list: LinkedList::new() }
    }

    /// 将睡眠节点插入到有序列表的正确位置
    pub fn add_sleeper(&mut self, new_node_arc: Arc<SleepNode>) {
        // LinkedList 没有直接的有序插入，我们需要手动查找位置
        let mut cursor = self.list.cursor_front_mut(); // std::collections::LinkedList::CursorMut

        // 找到第一个 deadline 大于 new_node_arc.deadline 的节点
        // 或者 deadline 相同但 task_ptr 大于 new_node_arc.task_ptr 的节点
        loop {
            match cursor.current() {
                Some(current_node_arc) => { // current_node_arc 是 &Arc<SleepNode>
                    if **current_node_arc > *new_node_arc { // 比较 SleepNode 内容
                        break; // 在此节点前插入
                    }
                }
                None => break, // 到达末尾或列表为空
            }
            cursor.move_next();
        }

        // cursor.insert_before() 将在当前元素（如果存在）之前插入，
        // 或者如果 cursor 在末尾（current() is None），则等效于 push_back。
        cursor.insert_before(new_node_arc);
        // 任务状态设置移至 SleepFuture::poll
    }

    /// 移除并返回列表中所有 deadline 小于等于 now 的节点
    pub fn pop_expired(&mut self, now: TimeVal) -> Vec<Arc<SleepNode>> {
        let mut expired_nodes = Vec::new();
        while let Some(front_node_arc) = self.list.front() { // front() 返回 Option<&Arc<SleepNode>>
            if (**front_node_arc).deadline <= now { // Deref Arc to get &SleepNode
                // 队首已到期，弹出
                if let Some(expired_node) = self.list.pop_front() { // pop_front() 返回 Option<Arc<SleepNode>>
                    expired_nodes.push(expired_node);
                }
            } else {
                // 队首未到期，由于列表有序，后续节点也不会到期
                break;
            }
        }
        expired_nodes
    }

    /// 从列表中移除指定的 Arc<SleepNode> 实例。
    /// 使用 Arc::ptr_eq 来精确匹配。
    pub fn remove_sleeper(&mut self, node_to_remove_arc: &Arc<SleepNode>) -> Option<Arc<SleepNode>> {
        let mut current_idx = 0;
        let mut found_idx = None;

        for node_in_list_arc in self.list.iter() { // iter() 返回 Iter<'_, Arc<SleepNode>>
                                                    // node_in_list_arc 是 &Arc<SleepNode>
            if Arc::ptr_eq(node_in_list_arc, node_to_remove_arc) {
                found_idx = Some(current_idx);
                break;
            }
            current_idx += 1;
        }

        if let Some(idx_to_remove) = found_idx {
            // LinkedList 没有直接按索引移除的方法。
            // 我们需要 split_off 来操作。
            // split_off(at) -> LinkedList<T>: Splits the list into two at the given index.
            // Returns a new list containing all elements from `at` to the end.
            // The original list contains elements from 0 to `at - 1`.
            let mut tail = self.list.split_off(idx_to_remove);
            // 此时，tail 的第一个元素是我们想移除的
            let removed_node = tail.pop_front(); // 移除并获取
            // 将 tail 中剩余的元素（如果有的话）重新接回 self.list
            self.list.append(&mut tail);
            return removed_node;
        }
        None
    }
}

/// 3. 全局睡眠队列
pub static GLOBAL_SLEEPER_QUEUE: Mutex<SleeperList> = Mutex::new(SleeperList::new());


/// 4. `sleep_until` 异步函数
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub fn sleep_until(deadline: TimeVal) -> SleepFuture {
    SleepFuture {
        deadline,
        task_ptr_at_creation: current_running_task_ptr(),
        // registered_node_arc 用于在 Drop 时精确移除
        // Option<Arc<SleepNode>> 表示 Future 持有对已注册节点的 Arc 引用
        registered_node_arc: None,
    }
}

/// 5. SleepFuture 实现
pub struct SleepFuture {
    deadline: TimeVal,
    task_ptr_at_creation: *const Task,
    // 存储对已注册到 GLOBAL_SLEEPER_QUEUE 中的 SleepNode 的 Arc 引用。
    // 当 Future drop 时，可以用这个 Arc 精确地从队列中移除对应的节点。
    registered_node_arc: Option<Arc<SleepNode>>,
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut_self = self.get_mut();

        if current_time() >= mut_self.deadline {
            if let Some(node_arc) = mut_self.registered_node_arc.take() {
                GLOBAL_SLEEPER_QUEUE.lock().remove_sleeper(&node_arc);
            }
            return Poll::Ready(());
        }

        if mut_self.registered_node_arc.is_some() {
            // 已注册，被唤醒 (非超时)
            mut_self.registered_node_arc.take(); // 从 Future 中移除 Arc，表示不再管理
            return Poll::Ready(());
        }

        // 首次 poll (未超时)
        let task_ptr_for_node = mut_self.task_ptr_at_creation;
        let waker = cx.waker().clone();
        let node = SleepNode::new(mut_self.deadline, waker, task_ptr_for_node);
        let node_arc = Arc::new(node); // 创建 Arc<SleepNode>

        // 设置任务状态
        let task = unsafe { &*task_ptr_for_node };
        let mut state_guard = task.state_lock_manual();
        if **state_guard == TaskStatus::Running || **state_guard == TaskStatus::Runable {
            **state_guard = TaskStatus::Blocking; // 或 TaskStatus::Sleeping
        }
        drop(state_guard);

        GLOBAL_SLEEPER_QUEUE.lock().add_sleeper(node_arc.clone()); // 将 Arc<SleepNode> 加入列表
        mut_self.registered_node_arc = Some(node_arc); // Future 持有这个 Arc 的克隆

        Poll::Pending
    }
}

impl Drop for SleepFuture {
    fn drop(&mut self) {
        // 如果 Future drop 时，它仍然持有一个 registered_node_arc，
        // 说明对应的 SleepNode 可能还在全局队列中，需要移除。
        if let Some(node_arc_to_remove) = self.registered_node_arc.take() {
            // 使用这个 Arc 引用去全局队列中精确查找并移除
            GLOBAL_SLEEPER_QUEUE.lock().remove_sleeper(&node_arc_to_remove);
        }
    }
}

/// 6. 定时事件处理
pub fn process_timed_events() {
    process_sleepers();
}

fn process_sleepers() {
    let now = current_time();
    let expired_nodes = GLOBAL_SLEEPER_QUEUE.lock().pop_expired(now);

    for node_arc in expired_nodes { // node_arc 是 Arc<SleepNode>
        node_arc.waker.wake(); // 调用 wakeup_task
    }
}
