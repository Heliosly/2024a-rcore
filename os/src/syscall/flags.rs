use crate::timer::TimeVal;



// For Mmap
bitflags! {
    /// Mmap permissions
    pub struct MmapProt: u32 {
        /// None
        const PROT_NONE = 0;
        /// Readable
        const PROT_READ = 1 << 0;
        /// Writable
        const PROT_WRITE = 1 << 1;
        /// Executable
        const PROT_EXEC = 1 << 2;
    }
}
bitflags! {
    pub struct FstatatFlags: usize {
        const SYMLINK_NO_FOLLOW = 0x100; // 不跟随符号链接
        const EMPTY_PATH      = 0x1000; // 允许空路径，表示操作 dirfd 本身
        const NO_AUTOMOUNT    = 0x800;  // 不自动挂载（可选）
        const REMOVEDIR       = 0x200;  // 仅用于 unlinkat
    }
}
bitflags::bitflags! {
    #[derive(Default)]
    pub struct MlockallFlags: u32 {
        /// 锁定当前进程中**所有已经映射的内存页**
        const MCL_CURRENT = 0x0001;
        /// 未来所有映射（mmap/brk）都自动锁定（如同自动 mlock）
        const MCL_FUTURE  = 0x0002;
    }
}

pub const  AT_FDCWD :i32=  -100;

bitflags! {
    /// Flags for the mremap system call.
    pub struct MremapFlags: u32 {
        /// By default, if there is not sufficient space at the current location,
        /// mremap() fails. With this flag, the kernel may move the mapping.
        const MAYMOVE     = 1 << 0;

        /// Together with MAYMOVE, force the mapping to be moved to the address
        /// specified by the optional new_address argument.
        const FIXED       = 1 << 1;

        /// Since Linux 5.7: remap to a new address but do not unmap the old range.
        const DONTUNMAP   = 1 << 2;
    }
}

bitflags! {
    /// 指定 sys_wait4 的选项
    pub struct WaitFlags: u32 {
        /// 不挂起当前进程，直接返回
        const WNOHANG = 1 << 0;
        /// 报告已执行结束的用户进程的状态
        const WIMTRACED = 1 << 1;
        /// 报告还未结束的用户进程的状态
        const WCONTINUED = 1 << 3;
        /// Wait for any child
        const WALL = 1 << 30;
        /// Wait for cloned process
        const WCLONE = 1 << 31;
    }
}
pub const F_DUPFD: usize = 0;
pub const F_DUPFD_CLOEXEC: usize = 1030;
pub const F_GETFD: usize = 1;
pub const F_SETFD: usize = 2;
pub const F_GETFL: usize = 3;
pub const F_SETFL: usize = 4;
pub const FD_CLOEXEC: usize = 1;

#[repr(C)] // 与 C iovec 兼容
#[derive(Debug, Copy, Clone)]
pub struct IoVec {
    pub base: *mut u8, // iov_base: Starting address of buffer
    pub len: usize,    // iov_len: Number of bytes to transfer to/from buffer
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Utsname {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
    pub domainname: [u8; 65],
}

bitflags! {
    pub struct FaccessatFileMode : u32 {
        const S_ISUID = 0o04000;
        const S_ISGID = 0o02000;
        const S_ISVTX = 0o01000;

        const S_IRUSR = 0o0400;
        const S_IWUSR = 0o0200;
        const S_IXUSR = 0o0100;
        const S_IRWXU = 0o0700;
        const S_IRGRP = 0o0040;
        const S_IWGRP = 0o0020;
        const S_IXGRP = 0o0010;
        const S_IRWXG = 0o0070;
        const S_IROTH = 0o0004;
        const S_IWOTH = 0o0002;
        const S_IXOTH = 0o0001;
        const S_IRWXO = 0o0007;
    }
}

bitflags! {
    pub struct FaccessatMode: u32 {
        const F_OK = 0;
        const X_OK = 1;
        const W_OK = 2;
        const R_OK = 4;
    }
}



#[repr(C)]
#[derive(Debug,  Clone, Copy)]
pub struct Rusage {
    pub ru_utime: TimeVal, // user CPU time used
    pub ru_stime: TimeVal, // system CPU time used
    // pub ru_maxrss: i64,    // maximum resident set size
    // pub ru_ixrss: i64,     // integral shared memory size
    // pub ru_idrss: i64,     // integral unshared data size
    // pub ru_isrss: i64,     // integral unshared stack size
    // pub ru_minflt: i64,    // page reclaims (soft page faults)
    // pub ru_majflt: i64,    // page faults (hard page faults)
    // pub ru_nswap: i64,     // swaps
    // pub ru_inblock: i64,   // block input operations
    // pub ru_oublock: i64,   // block output operations
    // pub ru_msgsnd: i64,    // IPC messages sent
    // pub ru_msgrcv: i64,    // IPC messages received
    // pub ru_nsignals: i64,  // signals received
    // pub ru_nvcsw: i64,     // voluntary context switches
    // pub ru_nivcsw: i64,    // involuntary context switches
}


#[repr(C)]
#[derive(Debug,Clone, Copy)]
pub struct Sysinfo {
    /// Seconds since boot
    pub uptime: usize,
    /// 1, 5, and 15 minute load averages
    pub loads: [usize; 3],
    /// Total usable main memory size
    pub totalram: usize,
    /// Available memory size
    pub freeram: usize,
    /// Amount of shared memory
    pub sharedram: usize,
    /// Memory used by buffers
    pub bufferram: usize,
    /// Total swap space size
    pub totalswap: usize,
    /// Swap space still available
    pub freeswap: usize,
    /// Number of current processes
    pub procs: u16,
    /// Total high memory size
    pub totalhigh: usize,
    /// Available high memory size
    pub freehigh: usize,
    /// Memory unit size in bytes
    pub mem_unit: u32,
}
extern "C" {
    fn _ekernel();
    fn _skernel();
}
impl Sysinfo {
    pub fn new(newuptime: usize, newtotalram: usize, newprocs: usize) -> Self {
        Self {
            uptime: newuptime,
            loads: [0; 3],
            totalram: newtotalram,
            freeram: newtotalram - (_ekernel as usize-_skernel as usize) ,
            sharedram: 0,
            bufferram: 0,
            totalswap: 0,
            freeswap: 0,
            procs: newprocs as u16,
            totalhigh: 0,
            freehigh: 0,
            mem_unit: 1,
        }
    }
}
// Futex options
pub const FUTEX_PRIVATE_FLAG: i32 = 128;
pub const FUTEX_CLOCK_REALTIME: i32 = 256;
pub const FUTEX_CMD_MASK: i32 = !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);

pub const FUTEX_TID_MASK: u32 = 1073741823;

pub const FUTEX_OWNER_DIED: u32 = 1073741824;
// Futex operations
pub const FUTEX_WAIT: i32 = 0;
pub const FUTEX_WAKE: i32 = 1;
pub const FUTEX_FD: i32 = 2;
pub const FUTEX_REQUEUE: i32 = 3;
pub const FUTEX_CMP_REQUEUE: i32 = 4;
pub const FUTEX_WAKE_OP: i32 = 5;
pub const FUTEX_LOCK_PI: i32 = 6;
pub const FUTEX_UNLOCK_PI: i32 = 7;
pub const FUTEX_TRYLOCK_PI: i32 = 8;
pub const FUTEX_WAIT_BITSET: i32 = 9;
pub const FUTEX_WAKE_BITSET: i32 = 10;
pub const FUTEX_WAIT_REQUEUE_PI: i32 = 11;
pub const FUTEX_CMP_REQUEUE_PI: i32 = 12;
pub const FUTEX_LOCK_PI2: i32 = 13;


pub const FUTEX_WAITERS: u32 = 2147483648;
// Futex flags
pub const FLAGS_SHARED: i32 = 0x10;
pub const FLAGS_CLOCKRT: i32 = 0x20;

// Futex bitset
pub const FUTEX_BITSET_MATCH_ANY: u32 = u32::MAX;
// FUTEX_WAKE_OP 的子操作
// 比较类型 (存放在 val3 的 24-27 位)
pub const FUTEX_OP_CMP_EQ: u32 = 0; // if (oldval == cmparg)
pub const FUTEX_OP_CMP_NE: u32 = 1; // if (oldval != cmparg)
pub const FUTEX_OP_CMP_LT: u32 = 2; // if (oldval < cmparg)
pub const FUTEX_OP_CMP_LE: u32 = 3; // if (oldval <= cmparg)
pub const FUTEX_OP_CMP_GT: u32 = 4; // if (oldval > cmparg)
pub const FUTEX_OP_CMP_GE: u32 = 5; // if (oldval >= cmparg)

// 操作类型 (存放在 val3 的 28-31 位)
pub const FUTEX_OP_SET: u32 = 0;   // oparg
pub const FUTEX_OP_ADD: u32 = 1;   // oldval + oparg
pub const FUTEX_OP_OR: u32 = 2;    // oldval | oparg
pub const FUTEX_OP_ANDN: u32 = 3;  // oldval & ~oparg
pub const FUTEX_OP_XOR: u32 = 4;   // oldval ^ oparg