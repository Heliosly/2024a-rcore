/// This file encapsulates the lwext4_rust interface and adapts it to the VFS.

///不支持并发 TODO(Heliosly)
use core::cell::RefCell;
use core::sync::atomic::AtomicBool;
use crate::alloc::string::String;
use crate::config::MAX_SYMLINK_DEPTH;
use crate::drivers::block::disk::Disk;
use crate::fs::inode::InodeType;
use crate::fs::stat::Kstat;
use crate::fs::vfs::vfs_ops::{VfsNodeOps, VfsOps};
use crate::fs::{as_inode_type, fix_path, OpenFlags, Statfs};
use crate::utils::error::{SysErrNo, SyscallRet};
use crate::utils::string::get_parent_path_and_filename;

use alloc::ffi::CString;
use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use log::*;
use lwext4_rust::bindings::{
     ext4_atime_set, ext4_ctime_set, ext4_mode_get, ext4_mode_set, ext4_mtime_set, ext4_owner_set, EOK, O_CREAT, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, SEEK_SET
};
use lwext4_rust::file::OsDirent;
use lwext4_rust::{Ext4BlockWrapper, Ext4File, InodeTypes};
use virtio_drivers::transport::Transport;
use virtio_drivers::Hal;



pub const BLOCK_SIZE: usize = 512;

#[allow(dead_code)]
pub struct Ext4FileSystem<H: Hal, T: Transport> {
   pub inner: Ext4BlockWrapper<Disk<H, T>>,
    root: Arc<dyn VfsNodeOps>,
    name:String,
}

unsafe impl<H: Hal, T: Transport> Sync for Ext4FileSystem<H, T> {}
unsafe impl<H: Hal, T: Transport> Send for Ext4FileSystem<H, T> {}

impl<H: Hal, T: Transport> Ext4FileSystem<H, T> {
    pub fn new(disk: Disk<H, T>,name:String,root:&str  ) -> Self {
        info!(
            "Got Disk size:{}, position:{}",
            disk.size(),
            disk.position()
        );
        let mut root_path = root.to_string();
        if !root_path.ends_with('/') {
            root_path.push('/');
        }
        let inner = Ext4BlockWrapper::<Disk<H, T>>::new(disk,name.clone(),&root_path)
            .expect("failed to initialize EXT4 filesystem");
        let root = Arc::new(FileWrapper::new(&root, InodeTypes::EXT4_DE_DIR));
        
        Self { inner, root ,name}
    }

}


/// The [`VfsOps`] trait provides operations on a filesystem.
impl<H: Hal, T: Transport> VfsOps for Ext4FileSystem<H, T> {
    fn mount(&mut self, path: &str, mount_point: Arc<dyn VfsNodeOps>) -> Result<usize, i32> {
        Ok(0)
    }
    fn name(&self)->String {
        self.name.clone()
    }

    fn root_inode(&self) -> Arc<dyn VfsNodeOps> {
        debug!("Get root_dir ops:{:?}",self.name);
        //let root_dir = unsafe { (*self.root.get()).as_ref().unwrap() };
        Arc::clone(&self.root)
    }


    fn ls(&self){
        self.inner
            .lwext4_dir_ls_with_vec()
            .into_iter()
            .for_each(|s| println!("{}", s));
    }
    
    fn statfs(&self) -> Result<Statfs,i32> {
        let stat = self.inner
        .get_statfs();



        Ok(Statfs {
            f_type: stat.f_type as i64,
            f_bsize: stat.f_bsize as i64,
            f_blocks: stat.f_blocks as i64,
            f_bfree: stat.f_bfree as i64,
            f_bavail: stat.f_bavail as i64,
            f_files: stat.f_files as i64,
            f_ffree: stat.f_ffree as i64,
            f_fsid: stat.f_fsid as i64,
            f_name_len: stat.f_name_len as i64,
            f_frsize: stat.f_frsize as i64,
            f_flags: stat.f_flags as i64,
            f_spare: stat.f_spare.map(|x| x as i64), // 如果是数组
        })

    }
   
}

pub struct FileWrapper{
    file:RefCell<Ext4File>,
    delay:AtomicBool,
}

unsafe impl Send for FileWrapper {}
unsafe impl Sync for FileWrapper {}

impl FileWrapper {
    pub fn new(path: &str, types: InodeTypes) -> Self {
        info!("FileWrapper new {:?} {}", types, path);
        //file.file_read_test("/test/test.txt", &mut buf);

        Self{file:RefCell::new(Ext4File::new(path, types)),delay:AtomicBool::new(false)}
    }

    fn path_deal_with(&self, path: &str) -> String {
        if path.starts_with('/') {
            warn!("path_deal_with: {}", path);
        }
        let p = path.trim_matches('/'); // 首尾去除
        if p.is_empty() || p == "." {
            return String::new();
        }

        if let Some(rest) = p.strip_prefix("./") {
            //if starts with "./"
            return self.path_deal_with(rest);
        }
        let rest_p = p.replace("//", "/");
        if p != rest_p {
            return self.path_deal_with(&rest_p);
        }

        //Todo ? ../
        //注：lwext4创建文件必须提供文件path的绝对路径
        let file = self.file.borrow_mut();
        let path = file.get_path();
        let fpath = String::from(path.to_str().unwrap().trim_end_matches('/')) + "/" + p;
        info!("dealt with full path: {}", fpath.as_str());
        fpath
    }
   
}

/// The [`VfsNodeOps`] trait provides operations on a file or a directory.
impl VfsNodeOps for FileWrapper {
    
    fn exchange(&self, path1: &str, path2: &str) -> Result<(), SysErrNo> {
        // 1. 从 path1 中拆出父目录和文件名
        //    比如 path1 = "/foo/bar/a.txt"，那么：
        //      parent1 = "/foo/bar"
        //      name1   = "a.txt"
        assert!(path1.starts_with('/'));

        assert!(path2.starts_with('/'));
        let (p1,name1) =  get_parent_path_and_filename(path1);
                  

        // 2. 构造一个临时文件名：确保它在 p1（也就是 path1 所在目录）下是唯一的
        //    这里我们简单用 “.swap_{name1}_{name2}” 作为临时名字；
        let (p2,name2) = get_parent_path_and_filename(path2);
        let tmp_name = format!(".swap_{}_{}", name1, name2);
        let temp_path = if p1 != "/" {
            let mut path = String::from(p1);
            path.push_str(&tmp_name);
            path
        } else {
            let mut path = String::from(p1);
            path.push_str(&tmp_name[1..]); // 去掉 tmp_name 的开头 '/'
            path
        };
           

        // 如果临时路径与 path1 或 path2 恰好相同，就报错（否则后面会覆盖原文件）
        if temp_path == path1 || temp_path == path2 {
            warn!("[impl VfsNodeOps for FileWrapper exchange] err: exchange path is same");
            return Err(SysErrNo::EEXIST); // 自定义一个错误码，或者使用 SysErrNo::EEXIST
        }


        // 步骤 1：把 path1 → temp_path
        //      如果这一步失败，就直接返回 Err，不做回滚
        self.rename(path1, &temp_path)
            .map_err(|e| {
                warn!("[exchange] step1: rename {} → {} failed: err={}", path1, temp_path, e);
                e
            })?;

        // 步骤 2：把 path2 → path1
        //      注意：此时 path1 已经移动到 temp_path 了，所以可以直接把 path2 改到 path1
        //      如果这一步失败，你可以选择回滚：把 temp_path 再改回 path1，但这里示例先不做回滚
        self.rename(path2, path1)
            .map_err(|e| {
                warn!("[exchange] step2: rename {} → {} failed: err={}", path2, path1, e);
                e
            })?;

        // 步骤 3：把 temp_path → path2
        //      如果这一步失败，同样可以选择回滚：把 path1 移回 path2，然后把 temp_path 移回 path1
        self.rename(&temp_path, path2)
            .map_err(|e| {
                warn!("[exchange] step3: rename {} → {} failed: err={}", temp_path, path2, e);
                e
            })?;

        // 三步都成功，则两个路径对应的节点已经交换完毕
        Ok(())
    }
    

 fn read_link(&self, buf: &mut [u8], bufsize: usize) -> SyscallRet {
        let file = &mut self.file.borrow_mut();
        file.file_readlink(buf, bufsize)
            .map_err(|e| SysErrNo::from(e))
    }
    fn delay(&self) {
        self.delay.store(true, core::sync::atomic::Ordering::Relaxed);
    }
    fn if_delay(&self) -> bool {
        
        self.delay.load(core::sync::atomic::Ordering::Acquire)

    }
    fn unlink(&self, path: &str) -> SyscallRet {
        let file = &mut self.file.borrow_mut();
        file.file_remove(path).map_err(|e| SysErrNo::from(e))
    }
    fn sym_link(&self, target: &str, path: &str) -> SyscallRet {
        let file = &mut self.file.borrow_mut();
        file.file_fsymlink(target, path)
            .map_err(|e| SysErrNo::from(e))
    }

    fn link_cnt(&self) -> SyscallRet {
        let file = &mut self.file.borrow_mut();
        let r = file.links_cnt();
        if let Err(e) = r {
            if e == 2 {
                return Ok(0);
            } else {
                return Err(SysErrNo::from(e));
            }
        }
        Ok(r.unwrap() as usize)
    }

    /*
    fn get_attr(&self) -> Result<usize, i32> {
        let mut file = self.file.lock();

        let perm = file.file_mode_get().unwrap_or(0o755);
        let perm = VfsNodePerm::from_bits_truncate((perm as u16) & 0o777);

        let vtype = file.file_type_get();
        let vtype = match vtype {
            InodeTypes::EXT4_INODE_MODE_FIFO => InodeType::Fifo,
            InodeTypes::EXT4_INODE_MODE_CHARDEV => InodeType::CharDevice,
            InodeTypes::EXT4_INODE_MODE_DIRECTORY => InodeType::Dir,
            InodeTypes::EXT4_INODE_MODE_BLOCKDEV => InodeType::BlockDevice,
            InodeTypes::EXT4_INODE_MODE_FILE => InodeType::File,
            InodeTypes::EXT4_INODE_MODE_SOFTLINK => InodeType::SymLink,
            InodeTypes::EXT4_INODE_MODE_SOCKET => InodeType::Socket,
            _ => {
                warn!("unknown file type: {:?}", vtype);
                InodeType::File
            }
        };

        let size = if vtype == InodeType::File {
            let path = file.get_path();
            let path = path.to_str().unwrap();
            file.file_open(path, O_RDONLY)
                .map_err(|e| <i32 as TryInto<AxError>>::try_into(e).unwrap())?;
            let fsize = file.file_size();
            let _ = file.file_close();
            fsize
        } else {
            0 // DIR size ?
        };
        let blocks = (size + (BLOCK_SIZE as u64 - 1)) / BLOCK_SIZE as u64;

        info!(
            "get_attr of {:?} {:?}, size: {}, blocks: {}",
            vtype,
            file.get_path(),
            size,
            blocks
        );

        Ok(VfsNodeAttr::new(perm, vtype, size, blocks))
    }
    */

    fn create(&self, path: &str, ty: InodeTypes) -> Result<usize, i32> {
        info!("create {:?} on Ext4fs: {}", ty, path);
        let fpath = self.path_deal_with(path);
        let fpath = fpath.as_str();
        if fpath.is_empty() {
            return Ok(0);
        }

        let types = ty;

        let mut file = self.file.borrow_mut();
        if file.check_inode_exist(fpath, types.clone()) {
            Ok(0)
        } else {
            if types == InodeTypes::EXT4_DE_DIR {
                file.dir_mk(fpath)
            } else {
                file.file_open(fpath, O_WRONLY | O_CREAT | O_TRUNC)
                    .expect("create file failed");
                file.file_close()
            }
        }
    }

    fn remove(&self, path: &str) -> Result<usize, i32> {
        info!("remove ext4fs: {}", path);
        let fpath = self.path_deal_with(path);
        let fpath = fpath.as_str();

        assert!(!fpath.is_empty()); // already check at `root.rs`

        let mut file = self.file.borrow_mut();
        if file.check_inode_exist(fpath, InodeTypes::EXT4_DE_DIR) {
            // Recursive directory remove
            file.dir_rm(fpath)
        } else {
            file.file_remove(fpath)
        }
    }

    /// Get the parent directory of this directory.
    /// Return `None` if the node is a file.
    fn parent(&self) -> Option<Arc<dyn VfsNodeOps>> {
        let file = self.file.borrow_mut();
        if file.get_type() == InodeTypes::EXT4_DE_DIR {
            let path = file.get_path();
            let path = path.to_str().unwrap();
            info!("Get the parent dir of {}", path);
            let path = path.trim_end_matches('/').trim_end_matches(|c| c != '/');
            if !path.is_empty() {
                return Some(Arc::new(Self::new(path, InodeTypes::EXT4_DE_DIR)));
            }
        }
        None
    }


        fn fstat(&self) -> Kstat {
            // 获取 ext4_inode_stat 结构
            let a= self.file.borrow_mut().fstat().unwrap();
        
            Kstat {
                st_dev: a.st_dev,
                st_ino: a.st_ino,
                st_mode: a.st_mode,
                st_nlink: a.st_nlink,
                st_uid: a.st_uid,
                st_gid: a.st_gid,
                st_rdev: 0,             // ext4_inode_stat 没有这个字段，填 0
                __pad: 0,               // 填 0
                st_size: a.st_size,
                st_blksize: a.st_blksize,
                __pad2: 0,              // 填 0
                st_blocks: a.st_blocks,
                st_atime: a.st_atime,
                st_atime_nsec: 0,       // ext4_inode_stat 没有纳秒，填 0
                st_mtime: a.st_mtime,
                st_mtime_nsec: 0,       // 填 0
                st_ctime: a.st_ctime,
                st_ctime_nsec: 0,       // 填 0
                __unused: [0; 2],       // 填 0
            }
        }
        

    
    
    /// Read directory entries into `dirents`, starting from `start_idx`.
    fn read_dentry(&self, off: usize, len: usize) -> Result<(Vec<u8>, isize), SysErrNo> {
        let file = &mut self.file.borrow();
        let entries = file
            .read_dir_from(off as u64)
            .map_err(|e| SysErrNo::from(e))?;
        let mut de: Vec<u8> = Vec::new();
        let (mut res, mut f_off) = (0usize, usize::MAX);
        for entry in entries {
            let dirent = crate::fs::Dirent {
                d_ino: entry.d_ino,
                d_off: entry.d_off,
                d_reclen: entry.d_reclen,
                d_type: entry.d_type,
                d_name: entry.d_name,
            };
            if res + dirent.len() > len {
                break;
            }
            res += dirent.len();
            f_off = dirent.off();
            de.extend_from_slice(dirent.as_bytes());
        }
        // (res != 0).then(|| (de, f_off as isize))
        assert!(res != 0);
        Ok((de, f_off as isize))
    }

    // /// Lookup the node with given `path` in the directory.
    // /// Return the node if found.
    // fn lookup(self: Arc<Self>, path: &str) -> Result<Arc<FileWrapper>, i32> {
    //     info!("lookup ext4fs: {:?}, {}", self.file.borrow().get_path(), path);

    //     let fpath = self.path_deal_with(path);
    //     let fpath = fpath.as_str();
    //     if fpath.is_empty() {
    //         return Ok(self.clone());
    //     }

    //     /////////
    //     let mut file = self.file;
    //     if file.check_inode_exist(fpath, InodeTypes::EXT4_DE_DIR) {
    //         debug!("lookup new DIR FileWrapper");
    //         Ok(Arc::new(Self::new(fpath, InodeTypes::EXT4_DE_DIR)))
    //     } else if file.check_inode_exist(fpath, InodeTypes::EXT4_DE_REG_FILE) {
    //         debug!("lookup new FILE FileWrapper");
    //         Ok(Arc::new(Self::new(fpath, InodeTypes::EXT4_DE_REG_FILE)))
    //     } else {
    //         Err(VfsError::NotFound)
    //     }
    // }
    

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SysErrNo> {
        // debug!("To read_at {}, buf len={}", offset, buf.len());
        let mut file = self.file.borrow_mut();
        let path = file.get_path();
        let path = path.to_str().unwrap();
        file.file_open(path, O_RDONLY)
            .map_err(|e| SysErrNo::from(e))?;
        file.file_seek(offset as i64, SEEK_SET)
            .map_err(|e| SysErrNo::from(e))?;
        let r = file.file_read(buf);
        r.map_err(|e| SysErrNo::from(e))
    }
    fn size(&self) -> usize {
        let file = &mut self.file.borrow_mut();
        let types = as_inode_type(file.get_type());
        if types == InodeType::File {
            let path = file.get_path();
            let path = path.to_str().unwrap();
            file.file_open(path, O_RDONLY).unwrap();
            let fsize = file.file_size();
            fsize as usize
        } else {
            0
        }
    }
    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, i32> {
        debug!("To write_at {}, buf len={}", offset, buf.len());
        let mut file = self.file.borrow_mut();
        let path = file.get_path();
        let path = path.to_str().unwrap();
        file.file_open(path, O_RDWR)?;

        file.file_seek(offset as i64, SEEK_SET)?;
        let r = file.file_write(buf);

        let _ = file.file_close();
        r
    }

    fn truncate(&self, size: u64) -> Result<usize, i32> {
        info!("truncate file to size={}", size);
        let mut file = self.file.borrow_mut();
        let path = file.get_path();
        let path = path.to_str().unwrap();
        file.file_open(path, O_RDWR | O_CREAT | O_TRUNC)?;

        let t = file.file_truncate(size);

        let _ = file.file_close();
        t
    }

    fn rename(&self, src_path: &str, dst_path: &str) -> Result<usize, i32> {
        info!("rename from {} to {}", src_path, dst_path);
        let mut file = self.file.borrow_mut();
        file.file_rename(src_path, dst_path)
    }
    fn find(
        &self,
        path: &str,
        flags: OpenFlags,
        loop_times: usize,
    ) -> Result<Arc<dyn VfsNodeOps>, crate::utils::error::SysErrNo> {
        // 先把所有对 file.borrow_mut() 的调用都做在一个独立的作用域里
        {
            let mut file = self.file.borrow_mut();
    
            if file.check_inode_exist(path, InodeTypes::EXT4_DE_DIR) {
                return Ok(Arc::new(FileWrapper::new(path, InodeTypes::EXT4_DE_DIR)));
            } else if file.check_inode_exist(path, InodeTypes::EXT4_DE_REG_FILE) {
                if flags.contains(OpenFlags::O_DIRECTORY) {
                    return Err(SysErrNo::ENOTDIR);
                }
                return Ok(Arc::new(FileWrapper::new(path, InodeTypes::EXT4_DE_REG_FILE)));
            } else if file.check_inode_exist(path, InodeTypes::EXT4_DE_SYMLINK) {
                if flags.contains(OpenFlags::O_ASK_SYMLINK) {
                    return Ok(Arc::new(FileWrapper::new(path, InodeTypes::EXT4_DE_SYMLINK)));
                }
                if loop_times >= MAX_SYMLINK_DEPTH {
                    return Err(SysErrNo::ELOOP);
                }
    
                // 读取链接目标到 buffer（注意这也要在这个作用域里完成）
                let mut file_name = [0u8; 256];
                let file_wrapper = FileWrapper::new(path, InodeTypes::EXT4_DE_SYMLINK);
                file_wrapper.read_link(&mut file_name, 256)?;
                let end = file_name.iter().position(|&v| v == 0).unwrap();
                let file_path = core::str::from_utf8(&file_name[..end]).unwrap();
                let prefix = path.rsplit_once("/").unwrap().0;
                let abs_path = format!("{}/{}", prefix, file_path);
    
                // `file` 这个 RefMut 在这里就会随着作用域结束而 drop，释放借用
                // 然后我们再递归调用 `find`
                return self.find(&abs_path, flags, loop_times + 1);
            }
        }
    
        // 到这里说明既不是目录也不是文件也不是 symlink
        Err(SysErrNo::ENOENT)
    }
  
    fn is_dir(&self) -> bool {
        self.file.borrow_mut().get_type() == InodeTypes::EXT4_DE_DIR
    }
fn set_owner(&self, uid: u32, gid: u32) -> SyscallRet {
    let file = self.file.borrow_mut();
    let c_path = file.get_path();
    let c_path = c_path.into_raw();

    let r = unsafe { ext4_owner_set(c_path, uid, gid) };

    unsafe {
        drop(CString::from_raw(c_path));
    }
    if r != EOK as i32 {
        error!("ext4_owner_set: rc = {}", r);
        return Err(r.into());
    }
    Ok(EOK as usize)
}

fn set_timestamps(
    &self,
    atime: Option<u32>,
    mtime: Option<u32>,
    ctime: Option<u32>,
) -> SyscallRet {
    trace!(
        "[set_timestamps] path = {}, atime = {:?}, mtime = {:?}, ctime = {:?}",
        self.path(),
        atime,
        mtime,
        ctime
    );
    let file = self.file.borrow_mut();
    let c_path = file.get_path();
    let c_path = c_path.into_raw();
    let mut r = 0;
    if let Some(atime) = atime {
        r = unsafe { ext4_atime_set(c_path, atime) }
    }
    if let Some(mtime) = mtime {
        r = unsafe { ext4_mtime_set(c_path, mtime) }
    }
    if let Some(ctime) = ctime {
        r = unsafe { ext4_ctime_set(c_path, ctime) }
    }
    unsafe {
        drop(CString::from_raw(c_path));
    }
    if r != EOK as i32 {
        error!("ext4_time_set: rc = {}", r);
        return Err(r.into());
    }
    Ok(EOK as usize)
}

fn fmode(&self) -> Result<u32, SysErrNo> {
    let file = self.file.borrow_mut();
    let c_path = file.get_path();
    let mut mode: u32 = 0o777;
    let c_path = c_path.into_raw();
    let r = unsafe { ext4_mode_get(c_path, &mut mode) };
    unsafe {
        drop(CString::from_raw(c_path));
    }
    if r != EOK as i32 {
        error!("ext4_mode_get: rc = {}", r);
        return Err(r.into());
    }
    Ok(mode)
}

fn fmode_set(&self, mode: u32) -> SyscallRet {
    let file = self.file.borrow_mut();
    let c_path = file.get_path();
    let c_path = c_path.into_raw();
    let r = unsafe { ext4_mode_set(c_path, mode) };
    unsafe {
        drop(CString::from_raw(c_path));
    }
    if r != EOK as i32 {
        error!("ext4_mode_set: rc = {}", r);
        return Err(r.into());
    }
    Ok(EOK as usize)
}
fn path (&self)->String{
    self.file.borrow().get_path().to_string_lossy().to_string()
}

}

impl Drop for FileWrapper {
    fn drop(&mut self) {
        let mut file = self.file.borrow_mut();
        debug!("Drop struct FileWrapper {:?}", file.get_path());
        file.file_close().expect("failed to close fd");
        drop(file); // todo
    }
}