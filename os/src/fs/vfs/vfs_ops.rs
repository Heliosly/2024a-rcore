
use alloc::{string::String, sync::Arc, vec::Vec};
use lwext4_rust::{bindings::ext4_direntry, InodeTypes};

use crate::{fs::{stat::Kstat, OpenFlags}, utils::error::{SysErrNo, SyscallRet}};

/// Filesystem operations.
pub trait VfsOps: Send + Sync {
    /// Do something when the filesystem is mounted.
    fn mount(&self, _path: &str, _mount_point: Arc<dyn VfsNodeOps>) -> Result<usize, i32> {
        Ok(0)
    }

    /// Do something when the filesystem is unmounted.
    fn umount(&self) -> Result<usize, i32> {
        Ok(0)
    }

    /// Format the filesystem.
    fn format(&self) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Get the attributes of the filesystem.
    fn statfs(&self) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Get the root directory of the filesystem.
    fn root_dir(&self) -> Arc<dyn VfsNodeOps>;
    
    fn ls(&self)  {
        unimplemented!()
    }
}

/// Node (file/directory) operations.
pub trait VfsNodeOps: Send + Sync {
    fn path(&self) ->String{
        unimplemented!()
    }
     fn fstat(&self)->Kstat{
        unimplemented!()
     }


     ///
     fn size(&self) -> usize {
        unimplemented!()
    }
    /// Do something when the node is opened.
    fn open(&self) -> Result<usize, i32> {
        Ok(0)
    }

    /// Do something when the node is closed.
    fn release(&self) -> Result<usize, i32> {
        Ok(0)
    }

    /// Get the attributes of the node.
    fn get_attr(&self) -> Result<usize, i32> {
        unimplemented!()
    }

    // file operations:

    /// Read data from the file at the given offset.
    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, i32> {
        unimplemented!()
    }
    fn read_link(&self, _buf: &mut [u8], _bufsize: usize) -> SyscallRet {
        unimplemented!()
    }
    fn sym_link(&self, _target: &str, _path: &str) -> SyscallRet {
        unimplemented!()
    }
    /// Write data to the file at the given offset.
    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Flush the file, synchronize the data to disk.
    fn fsync(&self) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Truncate the file to the given size.
    fn truncate(&self, _size: u64) -> Result<usize, i32> {
        unimplemented!()
    }

    // directory operations:

    /// Get the parent directory of this directory.
    ///
    /// Return `None` if the node is a file.
    fn parent(&self) -> Option<Arc<dyn VfsNodeOps>> {
        None
    }

    /// Lookup the node with given `path` in the directory.
    ///
    /// Return the node if found.
    fn lookup(self: Arc<Self>, _path: &str) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Create a new node with the given `path` in the directory
    ///
    /// Return [`Ok(())`](Ok) if it already exists.
    fn create(&self, _path: &str, _ty: InodeTypes) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Remove the node with the given `path` in the directory.
    fn remove(&self, _path: &str) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Read directory entries into `dirents`, starting from `start_idx`.
    fn read_dentry(&self, off: usize, len: usize) -> Result<(Vec<u8>, isize), SysErrNo>{
        unimplemented!()
    }
    /// Renames or moves existing file or directory.
    fn rename(&self, _src_path: &str, _dst_path: &str) -> Result<usize, i32> {
        unimplemented!()
    }

    /// Convert `&self` to [`&dyn Any`][1] that can use
    /// [`Any::downcast_ref`][2].
    ///
    /// [1]: core::any::Any
    /// [2]: core::any::Any#method.downcast_ref
    fn as_any(&self) -> &dyn core::any::Any {
        unimplemented!()
    }

    fn find(
        &self,
        path: &str,
        flags: OpenFlags,
        loop_times: usize,
    ) ->Result<Arc<dyn VfsNodeOps>, SysErrNo> {
        unimplemented!()
    }
    ///获取文件的mode，遇到需要文件访问权限的需要使用，暂时放在这里
    fn fmode(&self) -> Result<u32, SysErrNo> {
        unimplemented!();
    }
    fn fmode_set(&self, _mode: u32) -> SyscallRet {
        unimplemented!()
    }
     ///
     fn set_owner(&self, _uid: u32, _gid: u32) -> SyscallRet {
        unimplemented!()
    }
    ///
    fn set_timestamps(
        &self,
        _atime: Option<u32>,
        _mtime: Option<u32>,
        _ctime: Option<u32>,
    ) -> SyscallRet {
        unimplemented!()
    }
    ///
    fn is_dir(&self) -> bool {
        unimplemented!()
    }


    fn link_cnt(&self) -> SyscallRet {
        unimplemented!()
    }
    fn unlink(&self, path: &str) -> SyscallRet {
        unimplemented!()
    }
    fn delay(&self) {
        unimplemented!()
    }
     fn if_delay(&self) -> bool {
        unimplemented!()
     }
}