use alloc::sync::Arc;
use alloc::collections::LinkedList;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use spin::mutex::Mutex as SpinMutex;

pub struct GeneralWaitWakerNode {
    waker: Waker,
}
impl GeneralWaitWakerNode {
    pub fn new(waker: Waker) -> Self { Self { waker } }
}
#[derive(Default)]
pub struct GeneralWaitTaskList {
    list: LinkedList<Arc<GeneralWaitWakerNode>>,
}
impl GeneralWaitTaskList {
    pub fn new() -> Self { Default::default() }
    pub fn add_waiter(&mut self, node_arc: Arc<GeneralWaitWakerNode>) { self.list.push_back(node_arc); }
    pub fn remove_waiter(&mut self, node_to_remove_arc: &Arc<GeneralWaitWakerNode>) -> Option<Arc<GeneralWaitWakerNode>> {
        let mut current_idx = 0;
        let mut found_idx = None;
        for node_in_list_arc in self.list.iter() {
            if Arc::ptr_eq(node_in_list_arc, node_to_remove_arc) {
                found_idx = Some(current_idx); break;
            }
            current_idx += 1;
        }
        if let Some(idx_to_remove) = found_idx {
            let mut tail = self.list.split_off(idx_to_remove);
            let removed_node = tail.pop_front();
            self.list.append(&mut tail);
            removed_node
        } else { None }
    }
    pub fn notify_one(&mut self) -> bool {
        if let Some(node_arc) = self.list.pop_front() {
            node_arc.waker.wake_by_ref(); true
        } else { false }
    }
}
#[derive(Default)]
pub struct WaitQueue {
    queue: SpinMutex<GeneralWaitTaskList>,
}
impl WaitQueue {
    pub fn new() -> Self { Self { queue: SpinMutex::new(GeneralWaitTaskList::new()) } }
    pub fn notify_one(&self) -> bool { self.queue.lock().notify_one() }
    pub fn wait_until<'a, F>(&'a self, condition: F) -> WaitUntilFuture<'a, F>
    where F: Fn() -> bool + Unpin + Send + Sync + 'a, // Send + Sync for F
    {
        WaitUntilFuture { wq: self, condition, registered_node_arc: None }
    }
}
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WaitUntilFuture<'a, F: Fn() -> bool + Unpin> {
    wq: &'a WaitQueue,
    condition: F,
    registered_node_arc: Option<Arc<GeneralWaitWakerNode>>,
}
impl<'a, F: Fn() -> bool + Unpin> Future for WaitUntilFuture<'a, F> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut_self = self.get_mut();
        if (mut_self.condition)() {
            if let Some(node_arc) = mut_self.registered_node_arc.take() {
                mut_self.wq.queue.lock().remove_waiter(&node_arc);
            }
            return Poll::Ready(());
        }
        if mut_self.registered_node_arc.is_none() {
            let node = GeneralWaitWakerNode::new(cx.waker().clone());
            let node_arc = Arc::new(node);
            mut_self.wq.queue.lock().add_waiter(node_arc.clone());
            mut_self.registered_node_arc = Some(node_arc);
        }
        Poll::Pending
    }
}
impl<'a, F: Fn() -> bool + Unpin> Drop for WaitUntilFuture<'a, F> {
    fn drop(&mut self) {
        if let Some(node_arc) = self.registered_node_arc.take() {
            self.wq.queue.lock().remove_waiter(&node_arc);
        }
    }
}
