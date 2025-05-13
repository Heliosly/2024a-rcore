use core::array::from_fn;

use alloc::sync::Arc;

use crate::sync::SyncUnsafeCell;

use super::{KSigAction, SIG_MAX_NUM};

pub struct SigTable {
    pub inner: SyncUnsafeCell<SigTableInner>,
}

impl SigTable {
    pub fn new() -> Self {
        Self {
            inner: SyncUnsafeCell::new(SigTableInner::new()),
        }
    }
    pub fn from_another(another: &Arc<SigTable>) -> Self {
        Self {
            inner: SyncUnsafeCell::new(SigTableInner::from_another(another.get_ref())),
        }
    }
    pub fn get_ref(&self) -> &SigTableInner {
        self.inner.get_unchecked_ref()
    }
    pub fn get_mut(&self) -> &mut SigTableInner {
        self.inner.get_unchecked_mut()
    }

    pub fn action(&self, signo: usize) -> KSigAction {
        self.get_ref().actions[signo]
    }
    pub fn set_action(&self, signo: usize, act: KSigAction) {
        self.get_mut().actions[signo] = act
    }
    pub fn exit_code(&self) -> i32 {
        self.get_ref().group_exit_code.unwrap()
    }
    pub fn is_exited(&self) -> bool {
        self.get_ref().group_exit_code.is_some()
    }
    pub fn not_exited(&self) -> bool {
        self.get_ref().group_exit_code.is_none()
    }
    pub fn set_exit_code(&self, exit_code: i32) {
        self.get_mut().group_exit_code = Some(exit_code)
    }
}

pub struct SigTableInner {
    actions: [KSigAction; SIG_MAX_NUM + 1],
    group_exit_code: Option<i32>,
}

impl SigTableInner {
    pub fn new() -> Self {
        Self {
            actions: from_fn(|signo| KSigAction::new(signo, false)),
            group_exit_code: None,
        }
    }
    pub fn from_another(other: &SigTableInner) -> Self {
        Self {
            actions: other.actions.clone(),
            group_exit_code: None,
        }
    }
}
