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

mod fs;
mod process;
mod signal;
mod other;
mod arch;
pub mod flags;
use flags::{IoVec, Utsname};
use crate::timer::UserTimeSpec;
use fs::*;
use process::*;
use other::*;

use arch::*;
use crate::{fs::{Kstat, PollFd}, signal::{SigAction, SigSet}, timer::TimeVal, utils::error::SyscallRet};


use signal::*;
/// handle syscall exception with `syscall_id` and other arguments
pub async  fn syscall(syscall_id: usize, args: [usize; 6]) -> SyscallRet {
  match syscall_id {
        SYSCALL_OPEN => sys_openat(args[0] as i32,args[1] as *const u8,args[2] as u32,args[3] as u32).await,
        SYSCALL_CLOSE => sys_close(args[0] as i32).await,
        SYSCALL_CLOCK_NANOSLEEP=> sys_clock_nanosleep(args[0],args[1],args[2] as *const UserTimeSpec,args[3] as *mut UserTimeSpec).await,
        
        SYSCALL_FSTAT => sys_fstat(args[0] , args[1] as *mut Kstat).await,
        SYSCALL_EXIT => sys_exit(args[0] as i32).await,
        // SYSCALL_FORK => sys_fork(),
       
        SYSCALL_GETUID=>sys_getuid(),
        SYSCALL_SETTIDADDRESS=>sys_settidaddress(args[0]),
        SYSCALL_EXITGROUP => sys_exitgroup(args[0] as i32).await,
        SYSCALL_WAITPID => sys_wait4(args[0] as isize, args[1] as *mut i32, args[2] as i32).await,
        // SYSCALL_GET_TIME => sys_get_time(args[0] as *mut TimeVal, args[1]).await,
        SYSCALL_GETTIMEOFDAY=>sys_gettimeofday(args[0] as *mut UserTimeSpec, args[1] as usize).await,
        SYSCALL_TASK_INFO => sys_task_info(args[0] as *mut TaskInfo),
       
        SYSCALL_BRK => sys_brk(args[0] ).await,
        // SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        // SYSCALL_SET_PRIORITY => sys_set_priority(args[0] as isize),
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
        SYSCALL_FSTATAT => sys_fstatat(args[0] as i32, args[1] as *const u8, args[2] as *mut Kstat, args[3]).await,
        SYSCALL_YIELD => sys_yield().await,
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2] as u32,args[3] as u32,args[4],args[5]).await,
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]).await,
        SYSCALL_UNAME => sys_uname(args[0] as  *mut Utsname).await,
        SYSCALL_IOCTL =>sys_ioctl(args[0], args[1], args[2]),
        SYSCALL_FCNTL=>sys_fcntl(args[0], args[1], args[2]).await,
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]).await,
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]).await,
        SYSCALL_SIGNALRET =>sys_sigreturn().await,
        // SYSCALL_KILL => sys_kill(args[0] as isize, args[1]),
        SYSCALL_TKILL => sys_tkill(args[0], args[1]).await,
        SYSCALL_WRITEV=>sys_writev(args[0] , args[1] as *const IoVec, args[2] as i32).await,
        SYSCALL_READV=>sys_readv(args[0], args[1] as *const IoVec, args[2] as i32).await,
        SYSCALL_PREAD64=>sys_pread64(args[0] as i32, args[1] as *mut u8, args[2], args[3] ).await,
        SYSCALL_LSEEK=>sys_lseek(args[0], args[1] as isize, args[2] as u32 ).await, 
        SYSCALL_PWRITE64=>sys_pwrite64(args[0], args[1] as *const u8, args[2], args[3]).await,
        SYSCALL_GETEUID=> sys_geteuid() ,
        SYSCALL_GETCWD =>sys_getcwd(args[0] as *mut u8, args[1]).await,
        // SYSCALL_TGKILL => sys_tgkill(args[0], args[1], args[2]),
        SYSCALL_PPOLL => sys_ppoll(args[0] as *mut PollFd, args[1] , args[2] as *const UserTimeSpec, args[3] as *const SigSet).await,
        SYSCALL_CHDIR => sys_chdir(args[0] as *const u8).await,
        SYSCALL_GETDENTS64 => sys_getdents64(args[0], args[1] as *mut u8, args[2]).await,
        SYSCALL_SETPGID => Ok(0),
        SYSCALL_GETPGID => Ok(0),
        SYSCALL_CLOCK_GETTIME => sys_clock_gettime(args[0] , args[1]).await,
        SYSCALL_GETTID => sys_gettid(),
        SYSCALL_FACCESSAT=>sys_faccessat(args[0] as i32,args[1] as *const u8,args[2] as u32,args[3]).await,
        SYSCALL_GETROBUSTLIST => sys_get_robust_list(args[0], args[1] as *mut usize, args[2] as *mut usize).await,
        SYSCALL_SETROBUSTLIST => sys_set_robust_list(args[0], args[1]).await,
        SYSCALL_MKDIRAT => sys_mkdirat(args[0] as i32, args[1] as *const u8, args[2] as u32).await,
        SYSCALL_DUP2=> sys_dup(args[0] as i32).await,
        SYSCALL_DUP3=> sys_dup3(args[0] as i32, args[1] as i32, args[2]  as  u32).await,
        SYSCALL_MOUNT => sys_mount(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *const u8,
            args[3] as u32,
            args[4] as *const u8
        ).await,
        SYSCALL_UMOUNT2 => sys_umount2(args[0] as *const u8, args[1] as u32).await,
        SYSCALL_UNLINKAT => sys_unlinkat(args[0] as i32, args[1] as *const u8, args[2] as u32).await,
        SYSCALL_PRLIMIT64=> sys_prlimit(args[0] , args[1] as u32, args[2] as *const RLimit, args[3] as *mut RLimit).await,
        SYSCALL_CLOCK_SETTIME=>     Err(crate::utils::error::SysErrNo::ENOSYS),
        SYSCALL_SYMLINKAT=>sys_symlinkat(args[0] as *const u8,args[1] as i32, args[2] as *const u8).await,
        SYSCALL_READLINKAT=>sys_readlinkat(args[0] as i32, args[1] as *const u8, args[2] as *mut u8, args[3]).await,
        SYSCALL_GETRANDOM=>sys_getrandom(args[0] as *mut u8, args[1], args[2] as u32).await,
        SYSCALL_MPROTECT=>sys_mprotect(args[0], args[1],args[2] ).await,
        
        
        
        
        
        
        
        
        
        
        _ =>{
             panic!("Unsupported syscall_id: {}", syscall_id);
            }
    }
    


}
