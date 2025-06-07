//! Synchronization and interior mutability primitives

mod up;

use alloc::collections::btree_map::BTreeMap;
use lazy_init::LazyInit;
use spin::mutex::Mutex as SpinMutex;
pub use up::UPSafeCell;
mod waitqueue;
pub mod futex;
mod mutex;
pub use mutex::{Mutex,MutexGuard};

use futex::FutexWaitQueue;



type FutexKey = (usize, usize); // (process_token, user_virtual_address)



pub fn init_futex_system() { 
    futex::GLOBAL_FUTEX_SYSTEM.init_by(SpinMutex::new(BTreeMap::new()));
}


// mod mutex_test;
// pub use mutex_test::{Mutex,MutexGuard};
