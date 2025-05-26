

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


pub const  AT_FDCWD :i32=  -100;



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
