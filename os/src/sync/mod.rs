//! Synchronization and interior mutability primitives

mod up;

pub use up::UPSafeCell;
mod mutex;
mod waitqueue;
pub use mutex::{Mutex,MutexGuard};