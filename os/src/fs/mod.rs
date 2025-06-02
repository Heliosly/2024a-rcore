//! File trait & inode(dir, file, pipe, stdin, stdout)

pub(crate) mod inode;
mod stdio;
mod dirent;
mod ext4;
mod vfs;
mod stat;
mod fd;
mod pipe;
mod poll;
pub mod dev;
pub mod mount;
use core::{any::Any, future::Future, panic, task::{Context, Poll, Waker}};
use alloc::vec::Vec;
use async_trait::async_trait;
use dev::{find_device, open_device_file, register_device};

use crate::{ mm::UserBuffer,  task::{ custom_noop_waker }, timer::get_time_ms, utils::{error::{ASyncRet, ASyscallRet, GeneralRet, SysErrNo, SyscallRet, TemplateRet}, string::{get_parent_path_and_filename, normalize_absolute_path}}};
use alloc::{format, string::{String, ToString}, sync::Arc, vec};
use ext4::EXT4FS;
use hashbrown::{HashMap, HashSet};
use inode::InodeType;
use lwext4_rust::{bindings::SEEK_END, InodeTypes};
use spin::{Lazy, RwLock};
pub use stat::Statfs;
pub use poll::PollRequest;
use vfs::vfs_ops::VfsNodeOps;
pub use vfs::vfs_ops::VfsOps;
pub use stat::Kstat;
pub use inode::OsInode;
pub use fd::{FileClass,FileDescriptor};
pub use poll::{PollFuture};
use alloc::boxed::Box;
pub use dirent::Dirent;

pub const DEFAULT_FILE_MODE: u32 = 0o666;
pub const DEFAULT_DIR_MODE: u32 = 0o777;
pub const NONE_MODE: u32 = 0;
bitflags! {
    pub struct PollEvents: u16 {
        /// 有数据可读（普通或优先级）
        const POLLIN     = 0x0001;

        /// 紧急数据可读（带外数据）
        const POLLPRI    = 0x0002;

        /// 可以写入数据而不会阻塞
        const POLLOUT    = 0x0004;

        /// 发生错误（无法恢复的错误）
        const POLLERR    = 0x0008;

        /// 对端关闭连接（挂断）
        const POLLHUP    = 0x0010;

        /// 请求的文件描述符不是一个打开的文件
        const POLLNVAL   = 0x0020;

        /// 读半关闭（对端关闭了写入），用于 epoll
        const POLLRDHUP  = 0x2000;
    }
}
#[repr(C)]
#[derive(Debug, Clone,Copy)]
pub struct PollFd {
    /// 文件描述符
    pub fd: i32,

    /// 请求的事件（bitflags 结构）
    pub events: PollEvents,

    /// 实际返回的事件（bitflags 结构）
    pub revents: PollEvents,
}
#[async_trait]
/// trait File for all file types
pub trait File: Send + Sync + Any {
    /// Reads data from the file into the provided buffer.
    /// The lifetime of the returned Future is bound by the lifetime of `buf`.
    async fn read<'a>( 
        &self,                 
        mut buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo>{
unimplemented!()

    } 

    /// Writes data from the provided buffer to the file.
    /// The lifetime of the returned Future is bound by the lifetime of `buf`.
    async fn write<'a>(
        &self,
        buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo>{
        unimplemented!()
    }

    /// whether the file is writable
     fn writable<'a>(&'a self) -> TemplateRet<bool> {
      unimplemented!();
    }
     fn readable<'a>(&'a self) -> TemplateRet<bool> {
        unimplemented!();
      }

    fn clear(&self){
        panic!("d");
    }
      /// 获得文件信息
      fn fstat(&self) -> Kstat{
        unimplemented!("not support!");
      }
    
      /// 设置偏移量,并非所有文件都支持
      fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
          unimplemented!("not support!");
      }

    fn as_any(&self) -> &dyn Any {
        unimplemented!();
    }
    ///
    /// # 参数
    /// * `requested_events`: 一个 `PollEvents` 位掩码，指定用户感兴趣的事件 (例如 `POLLIN`)。
    /// * `waker_to_register`: 一个 `Waker` 对象的引用。如果请求的事件当前未就绪，
    ///   并且 `stdin` 支持对这些事件的异步等待，则应克隆此 `Waker` 并注册它，
    ///   以便在事件就绪时可以唤醒调用者 (通常是 `PollFuture`)。
    ///
    /// # 返回
    /// 一个 `PollEvents` 位掩码，指示哪些请求的事件（或错误/特殊事件）当前已就绪。
 
    fn poll(&self, events: PollEvents, waker_to_register: &Waker) -> PollEvents{
        unimplemented!()
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

pub use inode:: OpenFlags;
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
fn create_file(abs_path: &str, flags: OpenFlags, mode: u32) -> Result<FileDescriptor, SysErrNo> {
    // 一定能找到,因为除了RootInode外都有父结点
    let parent_dir = root_inode();
    let (readable, writable) = flags.read_write();
     parent_dir.create(abs_path, as_ext4_de_type(flags.node_type()))?;
    let inode= parent_dir.find(abs_path,flags,0)?;
    inode.fmode_set(mode)?;
    inode.set_owner(0, 0)?;
    inode.set_timestamps(None, Some((get_time_ms() / 1000) as u32), None)?;
    insert_inode_idx(abs_path, inode.clone());
    let osinode = OsInode::new(readable, writable, inode);
    Ok(FileDescriptor::new(flags, FileClass::File(Arc::new(osinode))))
}


/// 判断是否是动态链接文件
pub fn is_dynamic_link_file(path: &str) -> bool {
    path.ends_with(".so") || path.contains(".so.")
}
pub fn find_inode(abs_path :&str, flags:OpenFlags)->Result<Arc<dyn VfsNodeOps>, SysErrNo>{
      trace!("[find_inode] abs_path={}", abs_path);
      root_inode().find(abs_path, flags, 0)
}
///open file
pub fn open_file(mut abs_path: &str, flags: OpenFlags, mode: u32) -> Result<FileDescriptor, SysErrNo> {

    log::debug!("[open] abs_path={}", abs_path);

    //判断是否是设备文件
    if find_device(abs_path) {
        let device = open_device_file(abs_path)?;
        return Ok(FileDescriptor{flags,file:FileClass::Abs(device)});
    }
    // 如果是动态链接文件,转换路径
    if is_dynamic_link_file(abs_path) {
     
        abs_path = map_dynamic_link_file(abs_path);
    }

    
    let abs_path = &fix_path(abs_path);

    // println!("open_file abs_path={},pid:{}", abs_path, current_task_may_uninit().map_or_else(|| 0, |f| f.get_pid()));
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
        let osfile = OsInode::new(readable, writable, inode.clone());
        if flags.contains(OpenFlags::O_APPEND) {
            osfile.lseek(0, SEEK_END )?;
        }
        if flags.contains(OpenFlags::O_TRUNC) {
            inode.truncate(0)?;
        }
        return Ok(FileDescriptor::new(flags,FileClass::File(Arc::new(osfile))));
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

pub fn map_dynamic_link_file( path: &str) -> &str {

            log::warn!("[map_dynamic] path={}",path);
    if !path.starts_with('/') { panic!("worth path") };
    if !DYNAMIC_PATH.contains(path) {
       
        for prefix in DYNAMIC_PREFIX.iter() {
            let full_path = format!("{}{}", prefix, path);
            log::info!("[map_dynamic] full_path={}", full_path);
            if DYNAMIC_PATH.contains(full_path.as_str()) {
                return full_path.leak();
            }
        }
    }
    
    path
}
static DYNAMIC_PREFIX: Lazy<Vec<&'static str>> =
    Lazy::new(|| vec![ "/glibc", "/musl"]);

static DYNAMIC_PATH: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "/musl/lib/dlopen_dso.so",
        "/musl/lib/tls_align_dso.so",
        "/musl/lib/tls_init_dso.so",
        "/musl/lib/libc.so",
        "/musl/lib/tls_get_new-dtv_dso.so",
        "/glibc/lib/dlopen_dso.so",
         "/glibc/lib/libc.so", 
         "/glibc/lib/tls_get_new-dtv_dso.so", 
         "/glibc/lib/ld-linux-riscv64-lp64.so.1",
    
         "/glibc/lib/ld-linux-riscv64-lp64d.so.1",
          "/glibc/lib/tls_align_dso.so", 
          "/glibc/lib/tls_init_dso.so"

    ]
    .into_iter()
    .collect()
});












//
const MOUNTS: &str = " ext4 / ext rw 0 0\n";
const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nnobody:x:1:0:nobody:/nobody:/bin/bash\n";
const MEMINFO: &str = r"
MemTotal:         944564 kB
MemFree:          835248 kB
MemAvailable:     873464 kB
Buffers:            6848 kB
Cached:            36684 kB
SwapCached:            0 kB
Active:            19032 kB
Inactive:          32676 kB
Active(anon):        128 kB
Inactive(anon):     8260 kB
Active(file):      18904 kB
Inactive(file):    24416 kB
Unevictable:           0 kB
Mlocked:               0 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 0 kB
Writeback:             0 kB
AnonPages:          8172 kB
Mapped:            16376 kB
Shmem:               216 kB
KReclaimable:       9960 kB
Slab:              17868 kB
SReclaimable:       9960 kB
SUnreclaim:         7908 kB
KernelStack:        1072 kB
PageTables:          600 kB
NFS_Unstable:          0 kB
Bounce:                0 kB
WritebackTmp:          0 kB
CommitLimit:      472280 kB
Committed_AS:      64684 kB
VmallocTotal:   67108863 kB
VmallocUsed:       15740 kB
VmallocChunk:          0 kB
Percpu:              496 kB
HugePages_Total:       0
HugePages_Free:        0
HugePages_Rsvd:        0
HugePages_Surp:        0
Hugepagesize:       2048 kB
Hugetlb:               0 kB
";
const ADJTIME: &str = "0.000000 0.000000 UTC\n";
const LOCALTIME: &str =
    "lrwxrwxrwx 1 root root 33 11月 18  2023 /etc/localtime -> /usr/share/zoneinfo/Asia/Shanghai\n";
const PRELOAD: &str = "";

pub async  fn create_init_files() -> GeneralRet {
    //创建/proc文件夹
    open_file(
        "/proc",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR | OpenFlags::O_DIRECTORY,
        DEFAULT_DIR_MODE,
    )?;
    //创建/proc/mounts文件系统使用情况
    let mountsfile = open_file(
        "/proc/mounts",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut mountsinfo = String::from(MOUNTS);
    let mut mountsvec = Vec::new();
    unsafe {
        let mounts = mountsinfo.as_bytes_mut();
        mountsvec.push(core::slice::from_raw_parts_mut(
            mounts.as_mut_ptr(),
            mounts.len(),
        ));
    }
    let mountbuf = UserBuffer::new(mountsvec);
    let mountssize = mountsfile.write(mountbuf).await?;
    debug!("create /proc/mounts with {} sizes", mountssize);
    //创建/proc/meminfo系统内存使用情况
    let memfile = open_file(
        "/proc/meminfo",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut meminfo = String::from(MEMINFO);
    let mut memvec = Vec::new();
    unsafe {
        let mem = meminfo.as_bytes_mut();
        memvec.push(core::slice::from_raw_parts_mut(mem.as_mut_ptr(), mem.len()));
    }
    let membuf = UserBuffer::new(memvec);
    let memsize = memfile.write(membuf).await?;
    debug!("create /proc/meminfo with {} sizes", memsize);
    //创建/dev文件夹
    open_file(
        "/dev",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR | OpenFlags::O_DIRECTORY,
        DEFAULT_DIR_MODE,
    )?;
    //注册设备/dev/rtc和/dev/rtc0
    register_device("/dev/rtc");
    register_device("/dev/rtc0");
    //注册设备/dev/tty
    register_device("/dev/tty");
    //注册设备/dev/zero
    register_device("/dev/zero");
    //注册设备/dev/numm
    register_device("/dev/null");
    //注册设备/dev/cpu_dma_latency
    register_device("/dev/cpu_dma_latency");
    //创建./dev/misc文件夹
    open_file(
        "/dev/misc",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR | OpenFlags::O_DIRECTORY,
        DEFAULT_DIR_MODE,
    )?;
    //注册设备/dev/misc/rtc
    register_device("/dev/misc/rtc");
    //创建/etc文件夹
    open_file(
        "/etc",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR | OpenFlags::O_DIRECTORY,
        DEFAULT_DIR_MODE,
    )?;
    //创建/etc/adjtime记录时间偏差
    let adjtimefile = open_file(
        "/etc/adjtime",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut adjtime = String::from(ADJTIME);
    let mut adjtimevec = Vec::new();
    unsafe {
        let adj = adjtime.as_bytes_mut();
        adjtimevec.push(core::slice::from_raw_parts_mut(adj.as_mut_ptr(), adj.len()));
    }
    let adjtimebuf = UserBuffer::new(adjtimevec);
    let adjtimesize = adjtimefile.write(adjtimebuf).await?;
    debug!("create /etc/adjtime with {} sizes", adjtimesize);

    //创建./etc/localtime记录时区
    let localtimefile = open_file(
        "/etc/localtime",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut localtime = String::from(LOCALTIME);
    let mut localtimevec = Vec::new();
    unsafe {
        let local = localtime.as_bytes_mut();
        localtimevec.push(core::slice::from_raw_parts_mut(
            local.as_mut_ptr(),
            local.len(),
        ));
    }
    let localtimebuf = UserBuffer::new(localtimevec);
    let localtimesize = localtimefile.write(localtimebuf).await?;
    debug!("create /etc/localtime with {} sizes", localtimesize);

    //创建/etc/passwd记录用户信息
    let passwdfile = open_file(
        "/etc/passwd",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut passwd = String::from(PASSWD);
    let mut passwdvec = Vec::new();
    unsafe {
        let wd = passwd.as_bytes_mut();
        passwdvec.push(core::slice::from_raw_parts_mut(wd.as_mut_ptr(), wd.len()));
    }
    let passwdbuf = UserBuffer::new(passwdvec);
    let passwdsize = passwdfile.write(passwdbuf).await?;
    debug!("create /etc/passwd with {} sizes", passwdsize);

    //创建/etc/ld.so.preload记录用户信息
    let preloadfile = open_file(
        "/etc/ld.so.preload",
        OpenFlags::O_CREATE | OpenFlags::O_RDWR,
        DEFAULT_FILE_MODE,
    )?
    .file()?;
    let mut preload = String::from(PRELOAD);
    let mut preloadvec = Vec::new();
    unsafe {
        let pre = preload.as_bytes_mut();
        preloadvec.push(core::slice::from_raw_parts_mut(pre.as_mut_ptr(), pre.len()));
    }
    let preloadbuf = UserBuffer::new(preloadvec);
    let preloadsize = preloadfile.write(preloadbuf).await?;
    debug!("create /etc/ld.so.preload with {} sizes", preloadsize);

    println!("create_init_files success!");
    Ok(())
}



pub fn init(){

    let fut=create_init_files();
    let mut pinned = Box::pin(fut);
    let waker = custom_noop_waker();
        let mut ctx = Context::from_waker(&waker);
    
        match pinned.as_mut().poll(&mut ctx) {
            Poll::Ready(res) => res.unwrap(),
            Poll::Pending => {
                panic!("KERNEL_ASSERTION_FAILURE: FileSystem::init returned Pending");
            }
        };
    root_inode().set_timestamps(Some(0), Some(0), Some(0)).unwrap();
}

