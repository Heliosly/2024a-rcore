use spin::{Mutex as SpinMutex, MutexGuard as SpinMutexGuard};
use core::ops::{Deref, DerefMut};

use crate::task::yield_now;

/// A simple async‐compatible Mutex built on top of a spin::Mutex.
pub struct Mutex<T> {
    inner: SpinMutex<T>,
}

pub struct MutexGuard<'a, T> {
    guard: SpinMutexGuard<'a, T>,
}

impl<T> Mutex<T> {
    /// Create a new Mutex wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinMutex::new(value),
        }
    }

    /// Try to acquire the lock immediately.
    /// Returns `Some(MutexGuard)` on success, or `None` if already locked.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner
            .try_lock()
            .map(|g| MutexGuard { guard: g })
    }

    /// Asynchronously acquire the lock.
    /// Will spin‐wait, yielding to the executor whenever the lock is busy.
    pub async fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if let Some(g) = self.try_lock() {
                return g;
            }
            // Let other tasks make progress before retrying
            yield_now().await;
        }
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &*self.guard
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut *self.guard
    }
}

// Optional: if you want to drop the guard and release the lock
// you don't need to do anything: dropping MutexGuard will
// automatically release the underlying spin::MutexGuard.