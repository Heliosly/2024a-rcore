use crate::{signal::signal::{SigAction, SigSet}, utils::error::SysErrNo};


pub fn sys_sigprocmask(how: usize, set: *const SigSet, oldset: *mut SigSet) -> isize {
    if how > 2 {
        return SysErrNo::EINVAL as isize;
    }
    // 正常执行逻辑
    0
}

pub fn sys_rt_sigaction(
    signo: usize,
    act: *const SigAction,
    old_act: *mut SigAction,
) -> isize {
0
}