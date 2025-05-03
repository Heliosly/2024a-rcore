//! File trait & inode(dir, file, pipe, stdin, stdout)

pub(crate) mod inode;
mod stdio;
mod ext4;
mod vfs;
mod stat;
mod fd;
use core::panic;

use crate::{mm::UserBuffer, timer::get_time_ms, utils::error::{SysErrNo, SyscallRet}};
use alloc::{format, string::{String, ToString}, sync::Arc};
use ext4::EXT4FS;
use fd::FileClass;
use hashbrown::HashMap;
use inode::InodeType;
use lwext4_rust::{bindings::SEEK_END, InodeTypes};
use spin::{Lazy, RwLock};
use stat::Kstat;
pub use stat::Statfs;
use vfs::vfs_ops::VfsNodeOps;
pub use vfs::vfs_ops::VfsOps;


/// trait File for all file types
pub trait File: Send + Sync {
    /// the file readable?
    fn readable(&self) -> bool;
    /// the file writable?
    fn writable(&self) -> bool;
    /// read from the file to buf, return the number of bytes read
    fn read(&self, buf: UserBuffer) -> usize;
    /// write to the file from buf, return the number of bytes written
    fn write(&self, buf: UserBuffer) -> usize;
    ///d
    fn clear(&self){
        panic!("d");
    }
      /// 获得文件信息
      fn fstat(&self) -> Kstat{
        unimplemented!("not support!");
      }
    
      /// 设置偏移量,并非所有文件都支持
      fn lseek(&self, _offset: isize, _whence: usize) -> SyscallRet {
          unimplemented!("not support!");
      }
}

/// The stat of a inode
#[repr(C)]
#[derive(Debug)]
pub struct Stat {
    /// ID of device containing file
    pub dev: u64,
    /// inode number
    pub ino: u64,
    /// file type and mode
    pub mode: StatMode,
    /// number of hard links
    pub nlink: u32,
    /// unused pad
    pub(crate) pad: [u64; 7],
}

bitflags! {
    /// The mode of a inode
    /// whether a directory or a file
    pub struct StatMode: u32 {
        /// null
        const NULL  = 0;
        /// directory
        const DIR   = 0o040000;
        /// ordinary regular file
        const FILE  = 0o100000;
    }
}

pub use inode::{ OSInode, OpenFlags};
pub use stdio::{Stdin, Stdout};


pub fn list_app(){
     EXT4FS.ls();
}
fn root_inode() -> Arc<dyn VfsNodeOps > {
   
    let root = EXT4FS.root_dir();
    root
}
pub fn fix_path(path: &str) -> String {
    let mut path = path.to_string();
    if !path.starts_with("/") {
        path = format!("/{}", path);
    }
    
    if path.ends_with("/") {
        path = path[0..path.len() - 1].to_string();
    }
    path
}
fn as_ext4_de_type(types: InodeType) -> InodeTypes {
    match types {
        InodeType::BlockDevice => InodeTypes::EXT4_DE_BLKDEV,
        InodeType::CharDevice => InodeTypes::EXT4_DE_CHRDEV,
        InodeType::Dir => InodeTypes::EXT4_DE_DIR,
        InodeType::Fifo => InodeTypes::EXT4_DE_FIFO,
        InodeType::File => InodeTypes::EXT4_DE_REG_FILE,
        InodeType::Socket => InodeTypes::EXT4_DE_SOCK,
        InodeType::SymLink => InodeTypes::EXT4_DE_SYMLINK,
        InodeType::Unknown => InodeTypes::EXT4_DE_UNKNOWN,
    }
}

fn as_inode_type(types: InodeTypes) -> InodeType {
    match types {
        InodeTypes::EXT4_INODE_MODE_FIFO | InodeTypes::EXT4_DE_FIFO => InodeType::Fifo,
        InodeTypes::EXT4_INODE_MODE_CHARDEV | InodeTypes::EXT4_DE_CHRDEV => InodeType::CharDevice,
        InodeTypes::EXT4_INODE_MODE_DIRECTORY | InodeTypes::EXT4_DE_DIR => InodeType::Dir,
        InodeTypes::EXT4_INODE_MODE_BLOCKDEV | InodeTypes::EXT4_DE_BLKDEV => InodeType::BlockDevice,
        InodeTypes::EXT4_INODE_MODE_FILE | InodeTypes::EXT4_DE_REG_FILE => InodeType::File,
        InodeTypes::EXT4_INODE_MODE_SOFTLINK | InodeTypes::EXT4_DE_SYMLINK => InodeType::SymLink,
        InodeTypes::EXT4_INODE_MODE_SOCKET | InodeTypes::EXT4_DE_SOCK => InodeType::Socket,
        _ => {
            warn!("unknown file type: {:?}", types);
            unreachable!()
        }
    }
}
fn create_file(abs_path: &str, flags: OpenFlags, mode: u32) -> Result<FileClass, SysErrNo> {
    // 一定能找到,因为除了RootInode外都有父结点
    let parent_dir = root_inode();
    let (readable, writable) = flags.read_write();
     parent_dir.create(abs_path, as_ext4_de_type(flags.node_type()))?;
    let inode= parent_dir.find(abs_path,flags,0)?;
    inode.fmode_set(mode);
    inode.set_owner(0, 0);
    inode.set_timestamps(None, Some((get_time_ms() / 1000) as u32), None);
    insert_inode_idx(abs_path, inode.clone());
    let osinode = OSInode::new(readable, writable, inode);
    Ok(FileClass::File(Arc::new(osinode)))
}


/// 判断是否是动态链接文件
pub fn is_dynamic_link_file(path: &str) -> bool {
    path.ends_with(".so") || path.contains(".so.")
}
///open file
pub fn open_file(abs_path: &str, flags: OpenFlags, mode: u32) -> Result<FileClass, SysErrNo> {
    // log::info!("[open] abs_path={}", abs_path);
   
    // 如果是动态链接文件,转换路径
    if is_dynamic_link_file(abs_path) {
     
     panic!("dynamic link file"); //
    }

    let mut inode: Option<Arc<dyn VfsNodeOps >> = None;
    // 同一个路径对应一个Inode
    if has_inode(abs_path) {
        inode = find_inode_idx(abs_path);
    } else {
        let found_res = root_inode().find(abs_path, flags, 0);
        if found_res.clone().err() == Some(SysErrNo::ENOTDIR) {
            return Err(SysErrNo::ENOTDIR);
        }
        if found_res.clone().err() == Some(SysErrNo::ELOOP) {
            return Err(SysErrNo::ELOOP);
        }
        if let Ok(t) = found_res {
            if !flags.contains(OpenFlags::O_ASK_SYMLINK) {
                //符号链接文件不加入idx
                insert_inode_idx(abs_path, t.clone());
            }
            inode = Some(t);
        }
    }
    if let Some(inode) = inode {
        if flags.contains(OpenFlags::O_DIRECTORY) && !inode.is_dir() {
            return Err(SysErrNo::ENOTDIR);
        }
        let (readable, writable) = flags.read_write();
        let osfile = OSInode::new(readable, writable, inode.clone());
        if flags.contains(OpenFlags::O_APPEND) {
            osfile.lseek(0, SEEK_END as usize)?;
        }
        if flags.contains(OpenFlags::O_TRUNC) {
            inode.truncate(0)?;
        }
        return Ok(FileClass::File(Arc::new(osfile)));
    }

    // 节点不存在
    if flags.contains(OpenFlags::O_CREATE) {
        return create_file(abs_path, flags, mode);
    }
    Err(SysErrNo::ENOENT)
}




pub static FD2NODE: Lazy<RwLock<HashMap<String, Arc<dyn VfsNodeOps>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub fn has_inode(path: &str) -> bool {
    FD2NODE.read().contains_key(path)
}

pub fn find_inode_idx(path: &str) -> Option<Arc<dyn VfsNodeOps>> {
    FD2NODE.read().get(path).map(|inode| Arc::clone(inode))
}

pub fn insert_inode_idx(path: &str, inode: Arc<dyn VfsNodeOps>) {
    FD2NODE.write().insert(path.to_string(), inode);
}

pub fn remove_inode_idx(path: &str) {
    FD2NODE.write().remove(path);
}

pub fn print_inner() {
    println!("{:#?}", FD2NODE.read().keys());
}
