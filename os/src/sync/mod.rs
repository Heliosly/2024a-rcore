//! Synchronization and interior mutability primitives

mod up;


pub use up::UPSafeCell;
mod waitqueue;
pub mod futex;
mod mutex;
pub use mutex::{Mutex,MutexGuard};



// mod mutex_test;
// pub use mutex_test::{Mutex,MutexGuard};
