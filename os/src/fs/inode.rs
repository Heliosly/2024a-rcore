use alloc::boxed::Box;
use alloc::string::String;
/// src/fs/os_inode.rs

use alloc::sync::Arc;
use alloc::vec::Vec;
use async_trait::async_trait;
use bitflags::bitflags;
use lwext4_rust::bindings::{off_t, SEEK_CUR, SEEK_END, SEEK_SET};

use super::ext4::ops::FileWrapper;
use super::stat::Kstat;
use super::vfs::vfs_ops::VfsNodeOps;
use super::File;
use crate::fs::dirent::Dirent;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use crate::utils::error::{ASyncRet, ASyscallRet, SysErrNo, SyscallRet, TemplateRet};
use core::pin::Pin;
use core::future::Future;
pub const DEFAULT_FILE_MODE: u32 = 0o666;
pub const DEFAULT_DIR_MODE: u32 = 0o777;
pub const NONE_MODE: u32 = 0;
// 定义一份打开文件的标志
bitflags! {
    pub struct OpenFlags: u32 {
        // 保留低 3 位表示访问模式
        const O_RDONLY      = 0;           // 只读
        const O_WRONLY      = 1;           // 只写
        const O_RDWR        = 2;           // 读写
        const O_ACCMODE     = 3;           // 文件访问模式的掩码（用于提取只读/只写/读写）

        const O_CREATE      = 0o100;       // 若文件不存在则创建
        const O_EXCL        = 0o200;       // 与 O_CREATE 同用时，若文件已存在则打开失败
        const O_NOCTTY      = 0o400;       // 如果路径名指向终端设备，不将其设置为控制终端
        const O_TRUNC       = 0o1000;      // 若文件存在并成功打开，则将其截断为长度 0
        const O_APPEND      = 0o2000;      // 以追加模式写入（所有写入都追加到文件末尾）
        const O_NONBLOCK    = 0o4000;      // 非阻塞模式
        const O_DSYNC       = 0o10000;     // 写操作时，等待数据物理写入完成（不包括元数据）
        const O_SYNC        = 0o4010000;   // 写操作时，等待数据与元数据物理写入完成
        const O_RSYNC       = 0o4010000;   // 同步读操作（通常与 O_SYNC 效果相同）
        const O_DIRECTORY   = 0o200000;    // 打开目标必须是目录
        const O_NOFOLLOW    = 0o400000;    // 如果路径是符号链接，则打开失败（不跟随符号链接）
        const FD_CLOEXEC     = 0o2000000;   // exec 调用时自动关闭该文件描述符
        const O_ASYNC       = 0o20000;     // 启用异步 I/O，I/O 事件会产生信号
        const O_DIRECT      = 0o40000;     // 尽可能绕过页缓存进行直接磁盘访问
        const O_LARGEFILE   = 0o100000;    // 允许打开大于 2GB 的文件（32 位系统相关）
        const O_NOATIME     = 0o1000000;   // 不更新文件访问时间
        const O_PATH        = 0o10000000;  // 仅打开目录本身，不进行实际访问，可用于遍历路径
        const O_TMPFILE     = 0o20200000;  // 创建一个匿名临时文件（不会出现在目录中）

        const O_ASK_SYMLINK = 0o400000000; // 自定义：用于识别是否访问符号链接本身（而不是其目标）
    }
}



/// 在 OS 里再包装一层 Inode 以实现 File trait
pub struct OsInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}

pub struct OSInodeInner {
    offset: usize,
    inode: Arc<dyn VfsNodeOps>,
}

impl OsInode {
    pub fn new(readable: bool, writable: bool, inode: Arc<dyn VfsNodeOps>) -> Self {
        OsInode {
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
    pub fn get_path(&self)->String{
        let inner = self.inner.exclusive_access();
       inner.inode.path()
    }
    pub fn is_dir(&self)->bool{
        let inner =self.inner.exclusive_access();
        inner.inode.is_dir()
    }
    pub fn read_at(&self,offset:usize, buf:&mut [u8])->SyscallRet{
        let mut inner = self.inner.exclusive_access();
        let n = inner.inode.read_at(offset as u64, buf)?;
        inner.offset += n;
        Ok(n)
    }
    pub fn write_at(&self,offset:usize, buf:&[u8])->SyscallRet{
        let mut inner = self.inner.exclusive_access();
        let n = inner.inode.write_at(offset as u64, buf)?;
        inner.offset += n;
        Ok(n)
    }
    pub fn read_dentry(&self, off: usize, len: usize) -> Result<(Vec<u8>, isize), SysErrNo> {
        let file = &mut self.inner.exclusive_access().inode;
        file.read_dentry(off, len)
    }
    pub fn offset(&self) -> usize {
        self.inner.exclusive_access().offset
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
#[async_trait]
/// 为 `crate::traits::File` 实现 read/write/clear
impl File for OsInode {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn clear(&self) {
        let _ = self.inner.exclusive_access().inode.truncate(0);
        
    }
   fn readable(&self) -> TemplateRet<bool> {
        Ok(self.readable)  
    }

   fn writable(&self) -> TemplateRet<bool> {
       Ok(self.writable) 
    }
    async fn read<'a>( 
        & self,                
        mut buf: UserBuffer<'a>  
    ) -> Result<usize, SysErrNo> {
       
            let mut inner = self.inner.exclusive_access();
                let mut total = 0;
            for slice in buf.buffers.iter_mut() {
                let n = inner.inode.read_at(inner.offset as u64, slice)?;
                if n == 0 { break; }
                inner.offset += n;
                total += n;
            }
            Ok(total)
        
    }

    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo> {
      
            let mut inner = self.inner.exclusive_access();
            let mut total = 0;
            for slice in buf.buffers.iter() {
                let n = inner.inode.write_at(inner.offset as u64, *slice)?;
                inner.offset += n;
                total += n;
            }
            Ok(total)
      
    }
    fn lseek(&self, offset: isize, whence: u32) -> crate::utils::error::SyscallRet {
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

     fn fstat(&self) -> super::stat::Kstat {
        self.inner.exclusive_access().inode.fstat()
    }

    }

