use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::collections:: LinkedList;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::cmp::Ordering;

use spin::mutex::Mutex;
use lazy_init::LazyInit; 

use crate::timer::{TimeVal, current_time};
use crate::task::{Task, TaskStatus, TaskRef,  TID2TC}; 
use crate::task::waker::waker_from_task ; 
use super::current_task_id;

// --- 全局睡眠队列 ---
pub static GLOBAL_SLEEPER_QUEUE: LazyInit<Mutex<SleeperList>> = LazyInit::new();

// 初始化函数，
pub fn init_sleeper_queue() {
    GLOBAL_SLEEPER_QUEUE.init_by(Mutex::new(SleeperList::new()));
}


/// 1. 睡眠节点：存储唤醒时间、Waker 和 Task ID
pub struct SleepNode {
    deadline: TimeVal,
    waker: Waker,     
    task_id: usize,   // 存储任务 ID，用于比较、排序和 Drop 时识别
}

impl SleepNode {
    /// 创建一个新的睡眠节点
    pub fn new(deadline: TimeVal, waker: Waker, task_id: usize) -> Self {
        Self {
            deadline,
            waker,
            task_id,
        }
    }
}

// SleepNode 的比较逻辑主要基于 deadline，然后是 task_id
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
        self.deadline.cmp(&other.deadline)
            .then_with(|| self.task_id.cmp(&other.task_id))
    }
}


/// 2. 睡眠列表：使用 alloc::collections::LinkedList 存储 Arc<SleepNode>
pub struct SleeperList {
    list: LinkedList<Arc<SleepNode>>,
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
                    if **current_node_arc > *new_node_arc {
                        break;
                    }
                }
                None => break,
            }
            cursor.move_next();
        }
        cursor.insert_before(new_node_arc);
    }

    pub fn pop_expired(&mut self, now: TimeVal) -> Vec<Arc<SleepNode>> {
        let mut expired_nodes = Vec::new();
        while let Some(front_node_arc) = self.list.front() {
            if (**front_node_arc).deadline <= now {
                if let Some(expired_node) = self.list.pop_front() {
                    expired_nodes.push(expired_node);
                }
            } else {
                break;
            }
        }
        expired_nodes
    }

    pub fn remove_sleeper(&mut self, node_to_remove_arc: &Arc<SleepNode>) -> Option<Arc<SleepNode>> {
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


/// 4. `sleep_until` 异步函数
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub fn sleep_until(deadline: TimeVal) -> SleepFuture {
    SleepFuture {
        deadline,
        task_id_at_creation: current_task_id(), // 捕获当前任务的 ID
        registered_node_arc: None,
    }
}

/// 5. SleepFuture 实现
pub struct SleepFuture {
    deadline: TimeVal,
    task_id_at_creation: usize, // 创建此 Future 时当前任务的 ID
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
            mut_self.registered_node_arc.take();
            return Poll::Ready(());
        }

        // 未到期且尚未注册：这是首次 poll (且未超时)
        let task_id_for_node = mut_self.task_id_at_creation;

        // 1. 从 TID2TC 获取 Arc<Task> 以创建 Waker 和设置状态
        let task_arc_for_waker_and_state: TaskRef = { // TaskRef is Arc<Task>
            let tid2tc_map = TID2TC.lock(); // 你提供的全局任务表
            match tid2tc_map.get(&task_id_for_node) {
                Some(task_ref) => task_ref.clone(), // 克隆 Arc 以便后续使用
                None => {
                    // 任务在 sleep_until 被 poll 之前就从 TID2TC 中移除了。
                    // 这通常意味着任务已退出或被清理。
                    // log::warn!("Task ID {} not found in TID2TC during SleepFuture::poll. Completing sleep early.", task_id_for_node);
                    return Poll::Ready(()); // 任务不存在，睡眠无意义
                }
            }
        };

        // 2. 从 Arc<Task> 获取 *const Task 以创建 Waker
        //    SAFETY: task_arc_for_waker_and_state 是一个有效的 Arc，所以其内部指针有效。
        //    这个裸指针的生命周期与 Waker 的生命周期相关联。
        //     Waker 实现不持有 Arc，所以由调用者（这里是 Waker 的使用者，即调度器）
        //    保证在调用 wake() 时指针仍然有效（或者 wakeup_task 内部能安全处理）。
        let task_ptr_for_waker: *const Task = Arc::as_ptr(&task_arc_for_waker_and_state);
        let waker_for_sleepnode =  waker_from_task(task_ptr_for_waker) ;

        // 3. 创建 SleepNode
        let node = SleepNode::new(mut_self.deadline, waker_for_sleepnode, task_id_for_node);
        let node_arc = Arc::new(node);

        // 4. 设置任务状态 (使用 task_arc_for_waker_and_state)
        let mut state_guard = task_arc_for_waker_and_state.state_lock_manual();
        if **state_guard == TaskStatus::Running || **state_guard == TaskStatus::Runable {
            **state_guard = TaskStatus::Blocked;
        } else if **state_guard == TaskStatus::Blocking {
            // 根据 wakeup_task，Blocking 会变 Waked。如果希望它能被 add_task，则需 Blocked
            **state_guard = TaskStatus::Blocked;
        }
        // 其他状态（Waked, Zombie）不应改变，或应报错。
        drop(core::mem::ManuallyDrop::into_inner(state_guard));

        // 5. 加入全局睡眠队列
        GLOBAL_SLEEPER_QUEUE.lock().add_sleeper(node_arc.clone());
        mut_self.registered_node_arc = Some(node_arc);

        Poll::Pending
    }
}

impl Drop for SleepFuture {
    fn drop(&mut self) {
        if let Some(node_arc_to_remove) = self.registered_node_arc.take() {
            GLOBAL_SLEEPER_QUEUE.lock().remove_sleeper(&node_arc_to_remove);
        }
    }
}

/// 6. 定时事件处理
pub fn process_timed_events() {
    process_sleepers();
}

fn process_sleepers() {
    // 确保队列已初始化
    if !GLOBAL_SLEEPER_QUEUE.is_init() {
        log::warn!("GLOBAL_SLEEPER_QUEUE not initialized in process_sleepers");
        return;
    }
    let now = current_time();
    let expired_nodes = GLOBAL_SLEEPER_QUEUE.lock().pop_expired(now);

    for node_arc in expired_nodes {
       
        node_arc.waker.clone().wake(); 
    }
}

