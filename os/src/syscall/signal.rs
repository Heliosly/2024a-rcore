// use crate::{signal::signal::{SigAction, SigSet}, utils::error::{SysErrNo, SyscallRet}};


// pub fn sys_sigprocmask(how: usize, set: *const SigSet, oldset: *mut SigSet) -> SyscallRet {
//     if how > 2 {
//         return Err( SysErrNo::EINVAL);
//     }
//     // 正常执行逻辑
//     Ok(0)
// }

// pub fn sys_rt_sigaction(
//     signo: usize,
//     act: *const SigAction,
//     old_act: *mut SigAction,
// ) -> SyscallRet {
// Ok(0)
// }