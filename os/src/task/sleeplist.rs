use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::collections::LinkedList;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::cmp::Ordering;

use spin::mutex::Mutex;
use lazy_init::LazyInit;

use crate::timer::{TimeVal, current_time}; // 假设 TimeVal 可比较, Ord
use crate::task::{Task, TaskStatus, TaskRef, TID2TC};
use crate::task::waker::waker_from_task;
use super::current_task_id;
use super::schedule::remove_task; // 假设 current_task_id() -> usize

// --- 全局睡眠队列 ---
pub static GLOBAL_SLEEPER_QUEUE: LazyInit<Mutex<SleeperList>> = LazyInit::new();

pub fn init_sleeper_queue() {
    GLOBAL_SLEEPER_QUEUE.init_by(Mutex::new(SleeperList::new()));
}

/// 1. 睡眠节点：存储唤醒时间 (可选)、Waker 和 Task ID
pub struct SleepNode {
    deadline: Option<TimeVal>, // 改为 Option<TimeVal>
    waker: Waker,
    task_id: usize,
}

impl SleepNode {
    pub fn new(deadline: Option<TimeVal>, waker: Waker, task_id: usize) -> Self {
        Self {
            deadline,
            waker,
            task_id,
        }
    }
}

// SleepNode 的比较逻辑：
// None (永不超时) 被视为比任何 Some(TimeVal) 都大 (排在最后)
// 两个 None deadline 根据 task_id 比较
// 两个 Some(TimeVal) deadline 根据 TimeVal 然后 task_id 比较
impl PartialEq for SleepNode {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.task_id == other.task_id
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
        match (self.deadline, other.deadline) {
            (Some(d1), Some(d2)) => d1.cmp(&d2).then_with(|| self.task_id.cmp(&other.task_id)),
            (Some(_), None) => Ordering::Less,    // Some is "earlier" than None (infinite)
            (None, Some(_)) => Ordering::Greater, // None is "later" than Some
            (None, None) => self.task_id.cmp(&other.task_id), // Both infinite, order by task_id
        }
    }
}


/// 2. 睡眠列表：存储 Arc<SleepNode>
pub struct SleeperList {
    list: LinkedList<Arc<SleepNode>>, // 内部仍然按上述 Ord 规则排序
}

impl SleeperList {
    pub fn new() -> Self {
        Self { list: LinkedList::new() }
    }

    pub fn add_sleeper(&mut self, new_node_arc: Arc<SleepNode>) {
        let mut cursor = self.list.cursor_front_mut();
        loop {
            match cursor.current() {
                Some(current_node_arc) => {
                    // 使用 SleepNode 的 Ord 实现进行比较
                    if **current_node_arc > *new_node_arc {
                        break; // new_node_arc 应该在 current_node_arc 之前
                    }
                }
                None => break, // 到达末尾，new_node_arc 是最大的，或列表为空
            }
            cursor.move_next();
        }
        cursor.insert_before(new_node_arc); // 在找到的位置之前插入
    }

    pub fn pop_expired(&mut self, now: TimeVal) -> Vec<Arc<SleepNode>> {
        let mut expired_nodes = Vec::new();
        while let Some(front_node_arc_ref) = self.list.front() { // front() 返回 &Arc<SleepNode>
            match front_node_arc_ref.deadline { // front_node_arc_ref 是 &Arc<SleepNode>
                Some(deadline_val) => {
                    if deadline_val <= now {
                        // 队首已到期，弹出
                        if let Some(expired_node) = self.list.pop_front() {
                            expired_nodes.push(expired_node);
                        } else {
                            break; // 不应发生，因为 front() 刚才是 Some
                        }
                    } else {
                        // 队首未到期 (Some(deadline) > now)，由于列表有序，后续节点也不会到期
                        break;
                    }
                }
                None => {
                    // 队首是永不超时的节点，由于列表有序，它和它后面的都不会因时间到期
                    break;
                }
            }
        }
        expired_nodes
    }

    pub fn remove_sleeper(&mut self, node_to_remove_arc: &Arc<SleepNode>) -> Option<Arc<SleepNode>> {
        // ... (与之前版本相同，使用 Arc::ptr_eq)
        let mut current_idx = 0;
        let mut found_idx = None;
        for node_in_list_arc in self.list.iter() {
            if Arc::ptr_eq(node_in_list_arc, node_to_remove_arc) {
                found_idx = Some(current_idx);
                break;
            }
            current_idx += 1;
        }
        if let Some(idx_to_remove) = found_idx {
            let mut tail = self.list.split_off(idx_to_remove);
            let removed_node = tail.pop_front();
            self.list.append(&mut tail);
            removed_node
        } else {
            None
        }
    }
}


/// 4. `sleep_until` 异步函数，现在接受 Option<TimeVal>
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub fn sleep_until(deadline: Option<TimeVal>) -> SleepFuture {
    SleepFuture {
        deadline,
        task_id_at_creation: current_task_id(),
        registered_node_arc: None,
    }
}

/// 5. SleepFuture 实现
pub struct SleepFuture {
    pub deadline: Option<TimeVal>, // 改为 Option
    task_id_at_creation: usize,
    registered_node_arc: Option<Arc<SleepNode>>,
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut_self = self.get_mut();

        if let Some(deadline_val) = mut_self.deadline {
            // 如果有截止时间，检查是否已到期
            if current_time() >= deadline_val {
                if let Some(node_arc) = mut_self.registered_node_arc.take() {
                    GLOBAL_SLEEPER_QUEUE.lock().remove_sleeper(&node_arc);
                }
                return Poll::Ready(());
            }
        }
        // 如果 deadline 是 None，则永不因时间到期，只有 Waker 被调用时才会 Ready

        if mut_self.registered_node_arc.is_some() {
            // 已注册，现在被 poll。这通常意味着 Waker 被调用了。
            // （超时情况已在上面处理，如果 deadline 是 Some）
            mut_self.registered_node_arc.take();
            return Poll::Ready(());
        }

        // 未到期 (如果 deadline 是 Some) 或永不超时 (deadline 是 None)，且尚未注册
        let task_id_for_node = mut_self.task_id_at_creation;

        let task_arc_for_waker_and_state: TaskRef = {
            let tid2tc_map = TID2TC.lock();
            match tid2tc_map.get(&task_id_for_node) {
                Some(task_ref) => task_ref.clone(),
                None => return Poll::Ready(()), // 任务不存在
            }
        };
        task_arc_for_waker_and_state.set_state(TaskStatus::Blocking);
        let task_ptr_for_waker: *const Task = Arc::into_raw(task_arc_for_waker_and_state.clone());
        let waker_for_sleepnode = unsafe { waker_from_task(task_ptr_for_waker) };
         
        // Waker 的 Drop 实现需要能正确处理来自 Arc::into_raw 的指针

        let node = SleepNode::new(mut_self.deadline, waker_for_sleepnode, task_id_for_node);
        let node_arc = Arc::new(node);

        // 设置任务状态

        let mut state_guard = task_arc_for_waker_and_state.state_lock_manual();
        if **state_guard == TaskStatus::Running || **state_guard == TaskStatus::Runnable {
            **state_guard = TaskStatus::Blocking;
        } else if **state_guard == TaskStatus::Blocking {
            **state_guard = TaskStatus::Blocking;
        }
        
        drop(core::mem::ManuallyDrop::into_inner(state_guard));


        GLOBAL_SLEEPER_QUEUE.lock().add_sleeper(node_arc.clone());
        mut_self.registered_node_arc = Some(node_arc);

        Poll::Pending
    }
}

impl Drop for SleepFuture {
    fn drop(&mut self) {
        if let Some(node_arc_to_remove) = self.registered_node_arc.take() {
            // 尝试从队列移除，如果移除失败（例如已被 process_sleepers 弹出），也没关系
            let _ = GLOBAL_SLEEPER_QUEUE.lock().remove_sleeper(&node_arc_to_remove);
            // 重要的：node_arc_to_remove (Arc<SleepNode>) 在这里 drop，
            // 它内部的 Waker 会被 drop。
            // Waker 的 drop 实现必须能正确处理来自 Arc::into_raw 的指针，
            // 通常是调用 Arc::from_raw 使引用计数正确。
        }
    }
}

/// 6. 定时事件处理
pub fn process_timed_events() {
    process_sleepers();
}

fn process_sleepers() {
    if !GLOBAL_SLEEPER_QUEUE.is_init() {
        log::warn!("GLOBAL_SLEEPER_QUEUE not initialized in process_sleepers");
        return;
    }
    let now = current_time();
    // pop_expired 只会弹出那些 deadline 是 Some(t) 且 t <= now 的节点
    let expired_nodes = GLOBAL_SLEEPER_QUEUE.lock().pop_expired(now);

    for node_arc in expired_nodes {
        // 只有因时间到期的节点才会被唤醒
        node_arc.waker.clone().wake();
    }
}
