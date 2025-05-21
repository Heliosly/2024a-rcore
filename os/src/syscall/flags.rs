bitflags! {
    /// Mmap flags
    pub struct MmapFlags: u32 {
        /// Share changes
        const MAP_SHARED = 1;
        /// Changes are private
        const MAP_PRIVATE = 1 << 1;
        /// Interpret addr exactly
        const MAP_FIXED = 1 << 4;
        /// Don't use a file
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't permit write
        const MAP_DENYWRITE = 1 << 11;
        /// Populate (prefault) page tables 
        const MAP_POPULATE = 1 << 13;
        /// Region grows down (like a stack)
        const MAP_STACK = 1 << 17;
        /// MAP_FIXED_NOREPLACE: Don't replace existing mapping
        const MAP_FIXED_NOREPLACE = 1 << 16;
    }
}

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
#[repr(C)]
pub struct UtsName {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
}

pub const  AT_FDCWD :isize =  -100;



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
#[derive(Copy, Clone, Debug, Default)]
pub struct UserTimeSpec { // 对应 struct timespec
    pub tv_sec: usize,  // seconds
    pub tv_nsec: usize, // nanoseconds (long)
}