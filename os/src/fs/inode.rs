use alloc::boxed::Box;
use alloc::string::String;
/// src/fs/os_inode.rs

use alloc::sync::Arc;
use alloc::vec::Vec;
use async_trait::async_trait;
use bitflags::bitflags;
use lwext4_rust::bindings::{ SEEK_CUR, SEEK_END, SEEK_SET};
use spin::Mutex;

use super::vfs::vfs_ops::VfsNodeOps;
use super::File;
use crate::mm::UserBuffer;
use crate::utils::error::{ GeneralRet, SysErrNo, SyscallRet, TemplateRet};
pub const DEFAULT_FILE_MODE: u32 = 0o666;
pub const DEFAULT_DIR_MODE: u32 = 0o777;
pub const NONE_MODE: u32 = 0;
// 定义一份打开文件的标志
bitflags! {
    pub struct OpenFlags: u32 {
        const O_RDONLY      = 0;
        const O_WRONLY      = 1;
        const O_RDWR        = 2;
        const O_ACCMODE     = 3;

        const O_CREATE      = 0o100;
        const O_EXCL        = 0o200;
        const O_NOCTTY      = 0o400;
        const O_TRUNC       = 0o1000;
        const O_APPEND      = 0o2000;
        const O_NONBLOCK    = 0o4000;
        const O_DSYNC       = 0o10000;
        const O_SYNC        = 0o4010000;
        const O_RSYNC       = 0o4010000;
        const O_DIRECTORY   = 0o200000;
        const O_NOFOLLOW    = 0o400000;
        const FD_CLOEXEC  = 0o2000000;
        const O_ASYNC       = 0o20000;
        const O_DIRECT      = 0o40000;
        const O_LARGEFILE   = 0o100000;
        const O_NOATIME     = 0o1000000;
        const O_PATH        = 0o10000000;
        const O_TMPFILE     = 0o20200000;

       
        const O_ASK_SYMLINK = 0x80000000; 
    }
}



/// 在 OS 里再包装一层 Inode 以实现 File trait
pub struct OsInode {
    readable: bool,
    writable: bool,
    pub inner: Mutex<OSInodeInner>,
}

pub struct OSInodeInner {
    offset: usize,
    pub inode: Arc<dyn VfsNodeOps>,
}

impl OsInode {
    pub fn new(readable: bool, writable: bool, inode: Arc<dyn VfsNodeOps>) -> Self {
        OsInode {
            readable,
            writable,
            inner:  Mutex::new(OSInodeInner { offset: 0, inode }) ,
        }
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.lock();
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
        let inner = self.inner.lock();
       inner.inode.path()
    }
    pub fn is_dir(&self)->bool{
        let inner =self.inner.lock();
        inner.inode.is_dir()
    }
    pub fn read_at(&self,offset:usize, buf:&mut [u8])->SyscallRet{
        let mut inner = self.inner.lock();
        let n = inner.inode.read_at(offset as u64, buf)?;
        inner.offset += n;
        Ok(n)
    }
    pub fn write_at(&self,offset:usize, buf:&[u8])->SyscallRet{
        let mut inner = self.inner.lock();
        let n = inner.inode.write_at(offset as u64, buf)?;
        inner.offset += n;
        Ok(n)
    }
    pub fn read_dentry(&self, off: usize, len: usize) -> Result<(Vec<u8>, isize), SysErrNo> {
        let file = &mut self.inner.lock().inode;
        file.read_dentry(off, len)
    }
    pub fn offset(&self) -> usize {
        self.inner.lock().offset
    }
    pub fn set_timestamps(&self,atime:Option<u32>,mtime:Option<u32>,ctime:Option<u32>)->SyscallRet{
        self.inner.lock().inode.set_timestamps(atime, mtime, ctime)
         
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
        let _ = self.inner.lock().inode.truncate(0);
        
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
       
            let mut inner = self.inner.lock();
                let mut total = 0;
            for slice in buf.buffers.iter_mut() {
                let n = inner.inode.read_at(inner.offset as u64, slice)?;
                if n == 0 { break; }
                inner.offset += n;
                total += n;
            }
            trace!("[read] off:{}",inner.offset);
            Ok(total)
        
    }

    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo> {
      
            let mut inner = self.inner.lock();
            let mut total = 0;
            for slice in buf.buffers.iter() {
                let n = inner.inode.write_at(inner.offset as u64, *slice)?;
                inner.offset += n;
                total += n;
            }
            
            trace!("[write] off:{}",inner.offset);
            Ok(total)
      
    }
    fn lseek(&self, offset: isize, whence: u32) -> crate::utils::error::SyscallRet {
        if whence > 2 {
            return Err(SysErrNo::EINVAL);
        }
        let mut inner = self.inner.lock();
        if whence == SEEK_SET {
            if offset<0 {
                
                warn!("[OsInode::lseek]err:SEEK_SET off < 0,off = {}",offset);
                return Err(SysErrNo::EINVAL);
            }
            inner.offset = offset as usize;
        } else if whence == SEEK_CUR {
            let newoff = inner.offset as isize + offset;
            if newoff < 0 {
                warn!("[OsInode::lseek]err:SEEK_CUR off < 0,off = {}",newoff);
                return Err(SysErrNo::EINVAL);
            }
            inner.offset = newoff as usize;
        } else if whence == SEEK_END {
            let newoff = inner.inode.size() as isize + offset;
            if newoff < 0 {
                
                warn!("[OsInode::lseek]err:SEEK_END off < 0,off = {}",newoff);
                return Err(SysErrNo::EINVAL);
            }
            inner.offset = newoff as usize;
        }
        Ok(inner.offset)
    }

     fn fstat(&self) -> super::stat::Kstat {
        self.inner.lock().inode.fstat()
    }

    }

