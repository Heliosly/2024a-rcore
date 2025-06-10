//! Implementation of syscalls - LoongArch Linux Compatible
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
/// chdir syscall
const SYSCALL_CHDIR: usize = 49;
/// open syscall
const SYSCALL_OPEN: usize = 56;
/// close syscall
const SYSCALL_CLOSE: usize = 57;
/// getdents64 syscall
const SYSCALL_GETDENTS64: usize = 61;
/// read syscall
const SYSCALL_READ: usize = 63;
/// write syscall
const SYSCALL_WRITE: usize = 64;
/// fstat syscall (newfstatat in LoongArch)
const SYSCALL_FSTAT: usize = 79;  // 改为 newfstatat
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// clock_gettime syscall (gettime)
const SYSCALL_CLOCK_GETTIME: usize = 113;
/// sched_yield syscall (yield)
const SYSCALL_SCHED_YIELD: usize = 124;
/// setpriority syscall
const SYSCALL_SET_PRIORITY: usize = 141;  // 修正为 141
/// getpid syscall
const SYSCALL_GETPID: usize = 172;
/// brk syscall (sbrk)
const SYSCALL_BRK: usize = 214;
/// munmap syscall
const SYSCALL_MUNMAP: usize = 215;
/// clone syscall (fork)
const SYSCALL_CLONE: usize = 220;
/// execve syscall (exec)
const SYSCALL_EXECVE: usize = 221;
/// mmap syscall
const SYSCALL_MMAP: usize = 222;
/// wait4 syscall (waitpid)
const SYSCALL_WAIT4: usize = 260;

/// spawn syscall (rCore specific)
const SYSCALL_SPAWN: usize = 400;
/// taskinfo syscall (rCore specific)
const SYSCALL_TASK_INFO: usize = 410;

// 新增龙芯 Linux 特有的系统调用
/// getcwd syscall
const SYSCALL_GETCWD: usize = 17;
/// ioctl syscall
const SYSCALL_IOCTL: usize = 29;
/// fcntl syscall
const SYSCALL_FCNTL: usize = 25;

mod fs;
mod process;

use fs::*;
use process::*;

use crate::fs::Stat;

/// handle syscall exception with `syscall_id` and other arguments
pub async fn syscall(syscall_id: usize, args: [usize; 4]) -> isize {
    match syscall_id {
        // 文件系统相关
        SYSCALL_OPEN => sys_open(args[1] as *const u8, args[2] as u32),
        SYSCALL_CLOSE => sys_close(args[0]),
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_FSTAT => sys_fstat(args[0], args[1] as *mut Stat),
        SYSCALL_LINKAT => sys_linkat(args[1] as *const u8, args[3] as *const u8),
        SYSCALL_UNLINKAT => sys_unlinkat(args[1] as *const u8),
        SYSCALL_CHDIR => sys_chdir(args[0] as *const u8),
        SYSCALL_GETCWD => sys_getcwd(args[0] as *mut u8, args[1]),
        SYSCALL_GETDENTS64 => sys_getdents64(args[0], args[1] as *mut u8, args[2]),
        
        // 进程管理相关
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_SCHED_YIELD => sys_yield().await,  // 改名但功能相同
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_CLONE => sys_fork(),  // fork 的 LoongArch 实现
        SYSCALL_EXECVE => sys_exec(args[0] as *const u8),
        SYSCALL_WAIT4 => sys_waitpid(args[0] as isize, args[1] as *mut i32),
        SYSCALL_SET_PRIORITY => sys_set_priority(args[0] as isize),
        
        // 内存管理相关
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2]),
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]),
        SYSCALL_BRK => sys_sbrk(args[0] as i32),  // brk 的实现
        
        // 时间相关
        SYSCALL_CLOCK_GETTIME => sys_get_time(args[1] as *mut TimeVal, args[0]),
        
        SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        SYSCALL_TASK_INFO => sys_task_info(args[0] as *mut TaskInfo),
        
        124 => sys_yield().await,  // 旧的 yield 编号
        169 => sys_get_time(args[0] as *mut TimeVal, args[1]),
        
        _ => {
            println!("Unsupported syscall_id: {} (args: [{:#x}, {:#x}, {:#x}, {:#x}])", 
                     syscall_id, args[0], args[1], args[2], args[3]);
            -1
        }
    }
}
