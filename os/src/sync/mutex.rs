// --- src/sync/mutex.rs ---
use core::cell::UnsafeCell;
use core::fmt;
use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use core::task::{Context, Poll};

use crate::sync::waitqueue::WaitQueue;
use crate::task::{current_task, current_task_id, current_task_id_may_uninit, yield_now};

#[must_use = "If unused, the lock will be immediately unlocked"]
pub struct Mutex<T: ?Sized> {
    wq: WaitQueue,
    owner_task_id: AtomicUsize,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a Mutex<T>,
    data_ptr: Option<*mut T>,
    // todo：存储获取锁时当前任务的 ID，用于 drop 时验证 (可选，但更安全)  @Heliosly.
}

unsafe impl<'a, T: ?Sized + Send> Send for MutexGuard<'a, T> {}

impl<T> Mutex<T> {
    #[inline(always)]
    pub fn new(data: T) -> Self {
        Self {
            wq: WaitQueue::new(),
            owner_task_id: AtomicUsize::new(0),
            data: UnsafeCell::new(data),
        }
    }
    #[inline(always)]
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {
    #[inline(always)]
    pub fn is_locked(&self) -> bool {
        self.owner_task_id.load(AtomicOrdering::Relaxed) != 0
    }

    /// 创建一个 Future 来尝试获取锁。
    /// 这个 Future 在 poll 时会使用 `current_task_id()`。
    pub fn lock(&self) -> MutexGuard<T> {
        MutexGuard {
            lock: self,
            data_ptr: None,
        }
    }

    #[inline(always)]
    pub fn try_lock(&self) -> Option<MutexGuard<T>> {
        let current_id = current_task_id(); // 直接使用 current_task_id()
        if current_id == 0 {
            panic!("Task ID for Mutex operations cannot be 0.");
        }

        match self.owner_task_id.compare_exchange(
            0,
            current_id,
            AtomicOrdering::Acquire,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => Some(MutexGuard {
                lock: self,
                data_ptr: Some(self.data.get()),
            }),
            Err(_) => None,
        }
    }
   
    /// # Safety
    /// 强制解锁。调用者必须确保这是安全的操作，例如当前任务确实是锁的拥有者。
    /// （通常在 MutexGuard::drop 内部调用，此时 Guard 的存在就暗示了所有权）
    pub unsafe fn force_unlock(&self) {
        let releaser_id = current_task_id_may_uninit(); // 获取当前尝试解锁的任务ID
        let previous_owner_id = self.owner_task_id.swap(0, AtomicOrdering::Release);

        // 只有当锁之前确实被持有 (previous_owner_id != 0)
        // 并且尝试解锁的不是锁的持有者 (previous_owner_id != releaser_id) 时，才 panic。
        // 如果 releaser_id == 0，这是一个无效的解锁者，也应该视为问题（除非 previous_owner 也是0）。
        if previous_owner_id != 0 && previous_owner_id != releaser_id {
            panic!(
                "Task '{}' attempted to force_unlock a Mutex owned by task '{}')",
                releaser_id, previous_owner_id
            );
        }
        // 如果 releaser_id 是 0，但锁之前被一个非0的ID持有，这也是一个问题
        if releaser_id == 0 && previous_owner_id != 0 {
            panic!(
                "Task with ID 0 attempted to force_unlock a Mutex owned by task ID: {}",
                previous_owner_id
            );
        }

        if previous_owner_id != 0 {
            // 如果锁之前确实被持有
            self.wq.notify_one(); // 唤醒一个在 WaitQueue 中等待的任务
        }
    }

    #[inline(always)]
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

// Default, Debug impls (保持不变)
impl<T: ?Sized + Default> Default for Mutex<T> {
    #[inline(always)]
    fn default() -> Self {
        Self::new(Default::default())
    }
}
impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.try_lock() {
            Some(guard) => write!(f, "Mutex {{ data: {:?} }}", &*guard),
            None => write!(f, "Mutex {{ <locked> }}"),
        }
    }
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        unsafe {
            match self.data_ptr {
                Some(ptr) => &*ptr, // 如果已经加锁，data_ptr 是 Some，直接解引用返回引用
                None => panic!("MutexGuard: Dereferenced before lock acquired need await"), // 未加锁时解引用，panic 防止未定义行为
            }
        }
    }
}

impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            match self.data_ptr {
                Some(ptr) => &mut *ptr, // 已加锁，返回可变引用
                None => {
                    panic!("MutexGuard: Mutable dereference before lock acquired")
                    // 未加锁时尝试可变访问，panic
                }
            }
        }
    }
}

impl<'a, T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.data_ptr {
            Some(_) => fmt::Debug::fmt(&**self, f), // 如果加锁成功，打印内部数据的 Debug 格式
            None => write!(f, "MutexGuard {{ <lock not acquired> }}"), // 否则提示“未加锁”
        }
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        if self.data_ptr.is_some() {
            unsafe { self.lock.force_unlock() } // 如果加锁成功，释放锁
        }
        // 若未加锁，无需释放
    }
}

impl<'a, T: ?Sized + 'a> Future for MutexGuard<'a, T> {
    type Output = Self;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let guard_mut_ref = self.get_mut();

        // 如果 data_ptr 已经是 Some，说明这个 MutexGuard 实例已经持有锁（通常来自于 try_lock 或者上一次 .poll() 返回的 Guard）。
        // 这时我们把它从旧的 Guard（self）里取出来，创建一个新的 MutexGuard 返回给调用者，并结束。
        // 这么做可以避免旧的 self 在 Drop 时把锁释放掉——只有真正被返回的那个 Guard 持有释放责任。
        if guard_mut_ref.data_ptr.is_some() {
            let acquired_data_ptr = guard_mut_ref.data_ptr.take();
            return Poll::Ready(MutexGuard {
                lock: guard_mut_ref.lock,
                data_ptr: acquired_data_ptr,
            });
        }

        let current_id = current_task_id_may_uninit();

        if current_id == 0 {
            panic!("Task ID from current_task_id() cannot be 0 for Mutex operations.");
        }

        loop {
            // info!("into blocked");
            match guard_mut_ref.lock.owner_task_id.compare_exchange_weak(
                0,
                current_id,
                AtomicOrdering::Acquire,
                AtomicOrdering::Relaxed,
            ) {
                Ok(_) => {
                    // 成功获取锁
                    guard_mut_ref.data_ptr = Some(guard_mut_ref.lock.data.get());
                    return Poll::Ready(MutexGuard {
                        // 返回一个新的 Guard 实例
                        lock: guard_mut_ref.lock,
                        data_ptr: guard_mut_ref.data_ptr.take(), // 应为 Some
                    });
                }
                Err(owner_id_in_mutex) => {
                    // 锁被其他任务持有
                    if owner_id_in_mutex == current_id {
                        panic!("Task '{}' recursive lock attempt on Mutex", current_id);
                    }

                    // 锁被占用，使用 WaitQueue 等待
                    // 闭包只捕获 &AtomicUsize，确保 Send + Sync
                    info!("Race condition in Mutex");
                    let mut wait_condition_future = guard_mut_ref.lock.wq.wait_until({
                        let owner_atomic_ref = &guard_mut_ref.lock.owner_task_id;
                        move || owner_atomic_ref.load(AtomicOrdering::Relaxed) == 0
                    });

                    let pin_wait_condition_future =
                        unsafe { Pin::new_unchecked(&mut wait_condition_future) };
                    // ready! 会将 cx 传递给 wait_condition_future.poll()
                    // WaitUntilFuture::poll 内部会使用 cx.waker().clone() 注册到 WaitQueue
                    core::task::ready!(pin_wait_condition_future.poll(cx));

                    // 如果执行到这里，说明等待完成，继续外层 loop 尝试获取锁。
                }
            }
        }
    }
}
