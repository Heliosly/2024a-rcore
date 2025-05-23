//! Implementation of syscalls
//!
//! The single entry point to all system calls, [`syscall()`], is called
//! whenever userspace wishes to perform a system call using the `ecall`
//! instruction. In this case, the processor raises an 'Environment call from
//! U-mode' exception, which is handled as one of the cases in
//! [`crate::trap::trap_handler`].
//!
//! For clarity, each single syscall is implemented as its own function, named
//! `sys_` then the name of the syscall. You can find functions like this in
//! submodules, and you should also implement syscalls this way.

/// unlinkat syscall
const SYSCALL_UNLINKAT: usize = 35;
/// linkat syscall
const SYSCALL_LINKAT: usize = 37;
/// open syscall
const SYSCALL_OPEN: usize = 56;
/// close syscall
const SYSCALL_CLOSE: usize = 57;
/// read syscall
const SYSCALL_READ: usize = 63;
/// write syscall
const SYSCALL_WRITE: usize = 64;
/// fstat syscall
const SYSCALL_FSTAT: usize = 80;
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// yield syscall
const SYSCALL_YIELD: usize = 124;

const SYSCALL_KILL: usize = 129;
/// kill syscall
const SYSCALL_TKILL: usize = 130;
/// tkill syscall
const SYSCALL_TGKILL: usize = 131;
/// setpriority syscall
const SYSCALL_SET_PRIORITY: usize = 140;
/// gettime syscall
const SYSCALL_GET_TIME: usize = 169;
/// getpid syscall
const SYSCALL_GETPID: usize = 172;
/// sbrk syscall
const SYSCALL_SBRK: usize = 214;
/// munmap syscall
const SYSCALL_MUNMAP: usize = 215;
/// fork syscall
// const SYSCALL_FORK: usize = 220;
/// exec syscall
const SYSCALL_EXEC: usize = 221;
/// mmap syscall
const SYSCALL_MMAP: usize = 222;
/// waitpid syscall
const SYSCALL_WAITPID: usize = 260;
/// spawn syscall
const SYSCALL_SPAWN: usize = 400;
/// taskinfo syscall
const SYSCALL_TASK_INFO: usize = 410;
/// clone 
const SYSCALL_CLONE:usize = 220;
///set tid address
const SYSCALL_SETTIDADDRESS :usize =96;
///get uid
const SYSCALL_GETUID :usize = 174;
///exit group
const SYSCALL_EXITGROUP :usize=  94;
///
const SYSCALL_LSEEK: usize = 62;
const SYSCALL_READV: usize = 65;
const SYSCALL_WRITEV: usize = 66;
const SYSCALL_PREAD64: usize = 67;
const SYSCALL_PWRITE64: usize = 68;
const SYSCALL_SIGPROCMASK :usize =135;
const SYSCALL_RT_SIGACTION :usize =134;
const SYSCALL_GETPPID:usize = 173;
const SYSCALL_UNAME:usize = 160;
const SYSCALL_FSTATAT :usize =79;
const SYSCALL_IOCTL :usize =29;
const SYSCALL_FCNTL:usize =25;
const SYSCALL_SIGNALRET:usize =139;
const SYSCALL_GETEUID:usize=175;
const SYSCALL_GETCWD:usize= 17;
const SYSCALL_PPOLL:usize = 73;
const SYSCALL_CHDIR:usize = 49;
const SYSCALL_GETDENTS64:usize=61;
const SYSCALL_GETPGID :usize = 155;
const SYSCALL_SETPGID :usize = 154;
mod fs;
mod process;
mod signal;
mod other;
pub mod flags;
use flags::{IoVec, UserTimeSpec};
use fs::*;
use process::*;
use other::*;

use crate::{fs::{Kstat, PollFd}, signal::{SigAction, SigSet}, timer::TimeVal, utils::error::SyscallRet};


use signal::*;
/// handle syscall exception with `syscall_id` and other arguments
pub async  fn syscall(syscall_id: usize, args: [usize; 6]) -> SyscallRet {
  match syscall_id {
        SYSCALL_OPEN => sys_openat(args[0] as isize,args[1] as *const u8,args[2] as u32,args[3] as u32).await,
        SYSCALL_CLOSE => sys_close(args[0]).await,
        SYSCALL_LINKAT => sys_linkat(args[1] as *const u8, args[3] as *const u8),
        SYSCALL_UNLINKAT => sys_unlinkat(args[1] as *const u8),
        
        SYSCALL_FSTAT => sys_fstat(args[0], args[1] as *mut Kstat).await,
        SYSCALL_EXIT => sys_exit(args[0] as i32).await,
        // SYSCALL_FORK => sys_fork(),
       
        SYSCALL_GETUID=>sys_getuid(),
        SYSCALL_SETTIDADDRESS=>sys_settidaddress(),
        SYSCALL_EXITGROUP => sys_exitgroup(args[0] as i32).await,
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32).await,
        SYSCALL_GET_TIME => sys_get_time(args[0] as *mut TimeVal, args[1]).await,
        SYSCALL_TASK_INFO => sys_task_info(args[0] as *mut TaskInfo),
       
        SYSCALL_SBRK => sys_sbrk(args[0] as i32).await,
        SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        SYSCALL_SET_PRIORITY => sys_set_priority(args[0] as isize),
        SYSCALL_SIGPROCMASK => sys_sigprocmask(
            args[0] as i32,
            args[1] as *const SigSet,
            args[2] as *mut SigSet,
        ).await,
    
        SYSCALL_RT_SIGACTION => sys_sigaction(
            args[0],
            args[1] as *const SigAction,
            args[2] as *mut SigAction,
        ).await,
        SYSCALL_GETPPID => sys_getppid(),
        SYSCALL_CLONE => sys_clone(
         args
        ).await,
        SYSCALL_EXEC => sys_execve(args[0] as *const u8,
        
            args[1] as *const usize,
            args[2] as *const usize
        
        
        ).await,
        SYSCALL_FSTATAT => sys_fstatat(args[0] as isize, args[1] as *const u8, args[2] as *mut Kstat, args[3]).await,
        SYSCALL_YIELD => sys_yield().await,
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2] as u32,args[3] as u32,args[4],args[5]).await,
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]).await,
        SYSCALL_UNAME => sys_uname(args[0]).await,
        SYSCALL_IOCTL =>sys_ioctl(args[0], args[1], args[2]),
        SYSCALL_FCNTL=>sys_fcntl(args[0], args[1], args[2]).await,
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]).await,
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]).await,
        SYSCALL_SIGNALRET =>sys_sigreturn().await,
        // SYSCALL_KILL => sys_kill(args[0] as isize, args[1]),
        SYSCALL_TKILL => sys_tkill(args[0], args[1]).await,
        SYSCALL_WRITEV=>sys_writev(args[0] , args[1] as *const IoVec, args[2] as i32).await,
        SYSCALL_READV=>sys_readv(args[0], args[1] as *const IoVec, args[2] as i32).await,
        SYSCALL_PREAD64=>sys_pread64(args[0], args[1] as *mut u8, args[2], args[3] ).await,
        SYSCALL_LSEEK=>sys_lseek(args[0], args[1] as isize, args[2] ).await, 
        SYSCALL_PWRITE64=>sys_pwrite64(args[0], args[1] as *const u8, args[2], args[3]).await,
        SYSCALL_GETEUID=> sys_geteuid() ,
        SYSCALL_GETCWD =>sys_getcwd(args[0] as *mut u8, args[1]).await,
        // SYSCALL_TGKILL => sys_tgkill(args[0], args[1], args[2]),
        SYSCALL_PPOLL => sys_ppoll(args[0] as *mut PollFd, args[1] , args[2] as *const UserTimeSpec, args[3] as *const SigSet).await,
        SYSCALL_CHDIR => sys_chdir(args[0] as *const u8).await,
        SYSCALL_GETDENTS64 => sys_getdents64(args[0], args[1] as *mut u8, args[2]).await,
        SYSCALL_SETPGID => Ok(0),
        SYSCALL_GETPGID => Ok(0),
        _ => panic!("Unsupported syscall_id: {}", syscall_id),
    }
    


}
