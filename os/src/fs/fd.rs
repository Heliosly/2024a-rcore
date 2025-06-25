
use core::cell::UnsafeCell;

use alloc::{
    sync::Arc,
    vec,
    vec::Vec,
};

use crate::utils::error::{TemplateRet, SysErrNo, SyscallRet};

use super::{  File,OsInode, OpenFlags, Stdin, Stdout};
use core::ops::{Deref, DerefMut};
/// 枚举类型，分为普通文件和抽象文件
/// 普通文件File，特点是支持更多类型的操作，包含seek, offset等
/// 抽象文件Abs，抽象文件，只支持File trait的一些操作
#[derive(Clone)]
pub enum FileClass {
    File(Arc<OsInode>),
    Abs(Arc<dyn File>),
}

impl FileClass {
    pub fn file(&self) -> Result<Arc<OsInode>, SysErrNo> {
        match self {
            FileClass::File(f) => Ok(f.clone()),
            FileClass::Abs(_) => {

                debug!("[FileClass::file] err:not is file");
                Err(SysErrNo::EINVAL)
            },
        }
    }
    pub fn abs(&self) -> Result<Arc<dyn File>, SysErrNo> {
        match self {
            FileClass::File(_) => {

                debug!("[FileClass::abs] err:not is abs");
                Err(SysErrNo::EINVAL)
            },
            FileClass::Abs(f) => Ok(f.clone()),
        } 
    }
    pub fn any(&self) -> Arc<dyn File> {
        match self {
            FileClass::File(f) => f.clone(),
            FileClass::Abs(f) => f.clone(),
        }
    }
}
#[derive(Clone)]
pub struct FileDescriptor {
    pub flags: OpenFlags,
    pub file: FileClass,
    
}


impl Deref for FileDescriptor {
    type Target = dyn File;

    fn deref(&self) -> &Self::Target {
        match &self.file {
            FileClass::File(inode_arc) => {
                &**inode_arc
            }
            FileClass::Abs(file_arc) => {
                &**file_arc
            }
        }
    }
}

impl DerefMut for FileDescriptor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            match &self.file {
                FileClass::File(f) => {
                    let arc_ptr = Arc::as_ptr(f) as *mut OsInode;
                    &mut *arc_ptr
                }
                FileClass::Abs(f) => {
                    let arc_ptr = Arc::as_ptr(f) as *mut dyn File;
                    &mut *arc_ptr
                }
            }
        }
    }
}

impl FileDescriptor {
    pub fn new(flags: OpenFlags, file: FileClass) -> Self {
        Self { flags, file }
    }
    pub fn default(file: FileClass) -> Self {
        Self {
            flags: OpenFlags::empty(),
            file,
        }
    }
    pub fn file(&self) -> Result<Arc<OsInode>, SysErrNo> {
        self.file.file()
    }
    pub fn abs(&self) -> Result<Arc<dyn File>, SysErrNo> {
        self.file.abs()
    }
    pub fn any(&self) -> Arc<dyn File> {
        self.file.any()
    }

    pub fn unset_cloexec(&mut self) {
        self.flags &= !OpenFlags::FD_CLOEXEC;
    }
    pub fn set_cloexec(&mut self) {
        self.flags |= OpenFlags::FD_CLOEXEC;
    }
    pub fn cloexec(&self) -> bool {
        self.flags.contains(OpenFlags::FD_CLOEXEC)
    }
    pub fn non_block(&self) -> bool {
        self.flags.contains(OpenFlags::O_NONBLOCK)
    }
    pub fn unset_nonblock(&mut self) {
        self.flags &= !OpenFlags::O_NONBLOCK;
    }
    pub fn set_nonblock(&mut self) {
        self.flags |= OpenFlags::O_NONBLOCK;
    }
    pub fn is_file(&self)->bool{
          if let  FileClass::File(_) = self.file {
            true
          }
          else{
            false
          }
    }
 pub fn is_abs(&self)->bool{
          if let  FileClass::Abs(_) = self.file{
            true
          }
          else{
            false
          }
    }
}

