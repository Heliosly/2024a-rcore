/// src/fs/os_inode.rs

use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::bitflags;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_END, SEEK_SET};

use super::vfs::vfs_ops::VfsNodeOps;
use super::File;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use crate::utils::error::SysErrNo;


/// 在 OS 里再包装一层 Inode 以实现 File trait
pub struct OSInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}

pub struct OSInodeInner {
    offset: usize,
    inode: Arc<dyn VfsNodeOps>,
}

impl OSInode {
    pub fn new(readable: bool, writable: bool, inode: Arc<dyn VfsNodeOps>) -> Self {
        OSInode {
            readable,
            writable,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buf = [0u8; 512];
        let mut out = Vec::new();
        loop {
            let n =inner.inode.read_at(inner.offset as u64, &mut buf).unwrap();
            if n == 0 { break; }
            inner.offset += n;
            out.extend_from_slice(&buf[..n]);
        }
        out
    }
}

// 定义一份打开文件的标志
bitflags! {
    pub struct OpenFlags: u32 {
        // reserve 3 bits for the access mode
        const O_RDONLY      = 0;           // Read only
        const O_WRONLY      = 1;           // Write only
        const O_RDWR        = 2;           // Read and write
        const O_ACCMODE     = 3;           // Mask for file access modes
        const O_CREATE       = 0o100;       // Create file if it doesn't exist
        const O_EXCL        = 0o200;       // Exclusive use flag
        const O_NOCTTY      = 0o400;       // Do not assign controlling terminal
        const O_TRUNC       = 0o1000;      // Truncate flag
        const O_APPEND      = 0o2000;      // Set append mode
        const O_NONBLOCK    = 0o4000;      // Non-blocking mode
        const O_DSYNC       = 0o10000;     // Write operations complete as defined by POSIX
        const O_SYNC        = 0o4010000;   // Write operations complete as defined by POSIX
        const O_RSYNC       = 0o4010000;   // Synchronized read operations
        const O_DIRECTORY   = 0o200000;    // Must be a directory
        const O_NOFOLLOW    = 0o400000;    // Do not follow symbolic links
        const O_CLOEXEC     = 0o2000000;   // Set close-on-exec
        const O_ASYNC       = 0o20000;     // Signal-driven I/O
        const O_DIRECT      = 0o40000;     // Direct disk access hints
        const O_LARGEFILE   = 0o100000;    // Allow files larger than 2GB
        const O_NOATIME     = 0o1000000;   // Do not update access time
        const O_PATH        = 0o10000000;  // Obtain a file descriptor for a directory
        const O_TMPFILE     = 0o20200000;  // Create an unnamed temporary file

        const O_ASK_SYMLINK    = 0o400000000;     //自用，用于识别可访问符号链接本身文件的系统调用
    }
}

impl OpenFlags {
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::O_WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }

    pub fn node_type(&self) -> InodeType {
        if self.contains(OpenFlags::O_DIRECTORY) {
            InodeType::Dir
        } else {
            InodeType::File
        }
    }
}
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InodeType {
    Unknown = 0o0,
    /// FIFO (named pipe)
    Fifo = 0o1,
    /// Character device
    CharDevice = 0o2,
    /// Directory
    Dir = 0o4,
    /// Block device
    BlockDevice = 0o6,
    /// Regular file
    File = 0o10,
    /// Symbolic link
    SymLink = 0o12,
    /// Socket
    Socket = 0o14,
}

impl InodeType {
    /// Tests whether this node type represents a regular file.
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File)
    }
    /// Tests whether this node type represents a directory.
    pub const fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }
    /// Tests whether this node type represents a symbolic link.
    pub const fn is_symlink(self) -> bool {
        matches!(self, Self::SymLink)
    }
    /// Returns `true` if this node type is a block device.
    pub const fn is_block_device(self) -> bool {
        matches!(self, Self::BlockDevice)
    }
    /// Returns `true` if this node type is a char device.
    pub const fn is_char_device(self) -> bool {
        matches!(self, Self::CharDevice)
    }
    /// Returns `true` if this node type is a fifo.
    pub const fn is_fifo(self) -> bool {
        matches!(self, Self::Fifo)
    }
    /// Returns `true` if this node type is a socket.
    pub const fn is_socket(self) -> bool {
        matches!(self, Self::Socket)
    }
}

/// 为 `crate::traits::File` 实现 read/write/clear
impl File for OSInode {
    fn clear(&self) {
        self.inner.exclusive_access().inode.truncate(0);
        
    }
    fn readable(&self) -> bool { self.readable }
    fn writable(&self) -> bool { self.writable }

    fn read(&self, mut ub: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in ub.buffers.iter_mut() {
            let n =inner.inode.read_at(inner.offset as u64, slice).unwrap();
            if n == 0 { break; }
            inner.offset += n;
            total += n;
        }
        total
    }

    fn write(&self, ub: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in ub.buffers.iter() {
            let n = inner.inode.write_at(inner.offset as u64, *slice).unwrap();
            inner.offset += n;
            total += n;
        }
        total
    }
    fn lseek(&self, offset: isize, whence: usize) -> crate::utils::error::SyscallRet {
        let whence = whence as u32;
        if whence > 2 {
            return Err(SysErrNo::EINVAL);
        }
        let mut inner = self.inner.exclusive_access();
        if whence == SEEK_SET {
            inner.offset = offset as usize;
        } else if whence == SEEK_CUR {
            let newoff = inner.offset as isize + offset;
            if newoff < 0 {
                return Err(SysErrNo::EINVAL);
            }
            inner.offset = newoff as usize;
        } else if whence == SEEK_END {
            let newoff = inner.inode.size() as isize + offset;
            if newoff < 0 {
                return Err(SysErrNo::EINVAL);
            }
            inner.offset = newoff as usize;
        }
        Ok(inner.offset)
    }
    }

