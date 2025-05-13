//! File and filesystem-related syscalls
use core::mem;

use alloc::collections::btree_map::BTreeMap;
use alloc::string::{String, ToString};
use lwext4_rust::bindings::EINVAL;

pub struct Ino{
    link:u32,
    ino:u64,
}
use crate::fs::{ find_inode, open_file, File, FileClass, FileDescriptor, Kstat, OSInode, OpenFlags, Stat, StatMode};
use crate::mm::{translated_byte_buffer, translated_refmut, translated_str, UserBuffer};
use crate::task::{current_process, current_task, current_token};
use crate::utils::error::SysErrNo;

use super::flags::{ FstatatFlags, AT_FDCWD, FD_CLOEXEC, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};
pub static mut UMAP:BTreeMap<usize,String>=BTreeMap::new();
pub static mut UMAP1:BTreeMap<String,usize>=BTreeMap::new();
pub static mut UMAP2:BTreeMap<String,String>=BTreeMap::new();
pub static mut ITOS:BTreeMap<String,Ino>=BTreeMap::new();

pub static mut IDX:u64=1;
pub async fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write,fd:{}", current_task().get_pid(),fd);
    let token = current_token();
    let proc = current_process();
    let fd_table= proc.fd_table.lock();
    if fd >= fd_table.len() {
        return -1;
    }
    if let Some(file) = &fd_table[fd] {
        if !file.any().writable().await.unwrap() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        file.any().write(UserBuffer::new(translated_byte_buffer(token, buf, len))).await.unwrap() as isize
    } else {
        -1
    }
}

pub async fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read,fd:{}", current_task().get_pid(),fd);
    let token = current_token();
    let proc = current_process();
    let fd_table= proc.fd_table.lock();
    if fd >= fd_table.len() {
        return -1;
    }
    if let Some(file) = &fd_table[fd] {
        let file = file.clone();
        if !file.any().readable().await.unwrap() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        // trace!("kernel: sys_read .. file.read");
       let res= file.any().read(UserBuffer::new(translated_byte_buffer(token, buf, len))).await.unwrap() as isize;

       res
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().get_pid());
    let proc = current_process();
    let token = current_token();
    let path1 = translated_str(token, path);
    let path;
    if unsafe {
        UMAP2.contains_key(&path1)
    }{
        {
            
            path = unsafe { UMAP2 [&path1].clone()};
           
        }
    }
    else {
        path =path1;
    }
    let mut flag=1;
    unsafe{ if  ITOS.contains_key(&path.clone()){
        flag= ITOS.get(&path).unwrap().link;
        }
        
    }
    if flag==0{
        unsafe { ITOS.remove(&path.clone()) };
        return -1;
    }
    let openflags= OpenFlags::from_bits(flags).unwrap();
    match open_file(path.as_str(), openflags, 0o777) {
        Ok(inode) => {
            let fd = proc.alloc_fd();
            let mut fd_table = proc.fd_table.lock();
           fd_table[fd] = Some(FileDescriptor::new(openflags,  inode));
    
            unsafe {
                UMAP.insert(fd, path.clone());
                UMAP1.insert(path.clone(), fd);
    
                if !ITOS.contains_key(&path) {
                    ITOS.insert(path.clone(), Ino {
                        link: 1,
                        ino: IDX,
                    });
                    IDX += 1;
                }
            }
    
            fd as isize
        },
        Err(e) => {
            println!("open_file failed: {:?}", e);
            -1
        }
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().get_pid());
    let proc = current_process();
    let mut fd_table = proc.fd_table.lock();
    if fd >= fd_table.len() {
        return -1;
    }
    if fd_table[fd].is_none() {
        return -1;
    }
    fd_table[fd].take();

    unsafe { UMAP1.remove(&UMAP[&fd]) };
    unsafe { UMAP.remove(&fd) };
  
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(_fd: usize, _st: *mut Stat) -> isize {
    trace!("kernel:pid[{}] sys_fstat NOT IMPLEMENTED", current_task().get_pid());
    let proc = current_process();
    let fd_table = proc.fd_table.lock();
    if _fd >= fd_table.len() {
        return -1;
    }
    if fd_table[_fd].is_none() {
        return -1;
    }
   
    let op=unsafe { UMAP [&_fd].clone()};
    let temp= unsafe {
       &ITOS[&op]
    };

    let stat=Stat{
        dev:0,
        ino:temp.ino,
        mode:StatMode::FILE,
        nlink:temp.link,
        pad:[0;7],
  };
    
      let atm = unsafe { core::slice::from_raw_parts(&stat as *const _ as *const u8, mem::size_of::<Stat>()) };
      let token=current_token();
      let bufs=translated_byte_buffer(token, _st as *mut u8, mem::size_of::<Stat>());
      let mut i=0;
      for buf in bufs{
          for bf in buf{
              *bf=atm[i];
              i+=1;
          }
          
      }
      0
         
    
 
    
}


pub fn sys_ioctl(_fd: usize, _cmd: usize, _arg: usize) -> isize{
    // 伪实现
   0
}

pub fn sys_fstatat(
    dirfd: isize,
    path_ptr: *const u8,
    kst: *mut Kstat,
    flags: usize,
) -> isize {
    let token = current_token();
    // 从用户态指针获取 Rust 字符串（可能为空）
    let path = translated_str(token, path_ptr);
    let proc = current_process();
    let flags= FstatatFlags::from_bits(flags).unwrap();
    // 1. 先处理 AT_EMPTY_PATH：如果允许空路径且 path 为空，就 stat dirfd 自身
    if flags.contains(FstatatFlags::EMPTY_PATH)  && path.is_empty() {
        // dirfd 必须不是 AT_FDCWD
        if dirfd == AT_FDCWD {
            return -(SysErrNo::EINVAL as isize);
        }
        let fd_table = proc.fd_table.lock();
        let file = fd_table
            .get(dirfd as usize)
            .and_then(|opt| opt.clone())
            .ok_or_else(|| -(SysErrNo::EBADF as isize)).unwrap();
        let inode = file.file()
            .expect("Not an inode file");
        *translated_refmut(token, kst)=   inode.fstat() ;
        return 0;
    }

    // 2. 计算完整路径
    let base_path = if path.starts_with('/') {
        String::new()
    } else if dirfd == AT_FDCWD {
        proc.cwd.lock().clone()
    } else {
        let fd_table = proc.fd_table.lock();
        let file = fd_table
            .get(dirfd as usize)
            .and_then(|opt| opt.clone())
            .ok_or_else(|| -(SysErrNo::EBADF as isize)).unwrap();
        let inode = file.file()
            .expect("Not an inode file");
        inode.get_path()
    };
    let full_path = base_path + &path;

    // 3. 根据 AT_SYMLINK_NOFOLLOW 决定是否跟随符号链接
    let mut open_flags = OpenFlags::O_RDONLY;
    if flags.contains(FstatatFlags::SYMLINK_NO_FOLLOW) {
        open_flags |= OpenFlags::O_ASK_SYMLINK;
    }

    // 4. 在 VFS 中查找对应 inode
    match find_inode(full_path.as_str(), open_flags) {
        Ok(inode) => {
            // 将 inode 的元数据写入用户提供的 Kstat 结构
           
                *translated_refmut(token, kst)=   inode.fstat() ;
            
            0
        }
        Err(err) => {
            // 根据不同错误返回不同 errno
           err as isize
        }
    }
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(_old_name: *const u8, _new_name: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_linkat NOT IMPLEMENTED", current_task().get_pid());
    let token = current_token();
    let op=translated_str(token, _old_name);
    let np=translated_str(token, _new_name);
    if op==np{
        return -1;
    }
    unsafe { UMAP2.insert(np,op.clone()) };
    
    if unsafe { ITOS.contains_key(&op) }{
      
        unsafe {
           ITOS.get_mut(&op).unwrap().link+=1;
          
        }
        0
    }
    else{
        -1
    }
    
   

  
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(_name: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED", current_task().get_pid());
    let token = current_token();
    let name=translated_str(token, _name);
    //println!("suc1");
    //println!("unlink:np=?{}",name);
    if unsafe { UMAP2.contains_key(&name) }{
         let op = unsafe { UMAP2 [&name].clone()};
         //println!("unlink:op=?{}",op);
         unsafe { UMAP2.remove(&name) };
         if unsafe { ITOS.contains_key(&op) }{
            unsafe {
               let c=ITOS.get_mut(&op).unwrap();
               //println!("suc1");
               if c.link>0{
          
                c.link-=1;
               }
               else {
                let fd= UMAP1.get(&op).unwrap() ;
            current_process().fd_table.lock().get(*fd).unwrap().clone().unwrap().file().unwrap().clear();
               }
            }
        }    
    
   
     return 0;
    }
    else 
    {
           let  c= unsafe { ITOS .get_mut(&name).unwrap()};
           if c.link>0{
          
            c.link-=1;
           }
           else{
             let fd=unsafe { UMAP1.get(&name).unwrap() };
            current_process().fd_table.lock().get(*fd).unwrap().clone().unwrap().file().unwrap().clear();

           }
          
           
               
    }
   0
}



/// 系统调用 fcntl 的实现
pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> isize {
    let proc = current_process();
    
    let mut table_guard = proc.fd_table.lock();
    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            // 校验 fd 是否有效
            let file = match proc.get_file(fd) {
                Some(f) => f,
                None => return SysErrNo::EBADF as isize,
            };

            // arg 是新描述符号的下限
            let min_fd = arg;

            // 分配从 min_fd 开始的空闲描述符
            match proc.alloc_fd_from(min_fd) {
                Some(new_fd) => {
                     // 如果 new_fd 超出了当前 fd_table 的长度，需要扩展它
        if new_fd >= table_guard.len() {
            // 用 None 填充，直到 new_fd 这个索引是有效的
            // resize_with 比循环 push 更高效
            table_guard.resize_with(new_fd + 1, || None);
        }
                 
                    // 复制文件对象（Arc 克隆）
                    table_guard[new_fd] = Some(file.clone());
                    // 如果是 F_DUPFD_CLOEXEC，则设置 FD_CLOEXEC
                    if cmd == F_DUPFD_CLOEXEC {
                       table_guard[new_fd].clone().unwrap().set_cloexec();
                    } else {
                        table_guard[new_fd].clone().unwrap().unset_cloexec() ; // 默认不设置 close-on-exec
                    }
                    new_fd as isize
                }
                None =>SysErrNo::EMFILE as isize, // 描述符已用尽
            }
        }
        F_GETFD => {
            // 校验 fd 是否有效
            if fd >= table_guard.len() || table_guard[fd].is_none() {
                return SysErrNo::EBADF as isize;
            }
            // 返回文件描述符标志
            table_guard[fd].clone().unwrap().cloexec() as isize
        }
        F_SETFD => {
            // 校验 fd 是否有效
            if fd >= table_guard.len() || table_guard[fd].is_none() {
                return SysErrNo::EBADF as isize;
            }
            // 仅保留 FD_CLOEXEC 位
            let mut fd = table_guard[fd].clone().unwrap();
             if arg & FD_CLOEXEC==1{
                fd.set_cloexec();
             }
             else {
                fd.unset_cloexec();
             }
            0 // 成功
        }
        F_GETFL => {
            // 校验 fd 是否有效
            let file = match proc.get_file(fd) {
                Some(f) => f,
                None => return SysErrNo::EBADF as isize,
            };
            // 获取文件状态标志（O_APPEND、O_NONBLOCK 等）
            match &table_guard[fd] {
                Some(fd) => fd.flags.bits() as isize,
                None => SysErrNo::EBADF as isize,
            }
        }
        F_SETFL => {
            // 校验 fd 是否有效
            let mut file = match proc.get_file(fd) {
                Some(f) => f,
                None => return SysErrNo::EBADF as isize,
            };

            // 获取当前标志
            let current_flags = file.flags;

            // 可修改的标志掩码（这里只允许 O_APPEND 和 O_NONBLOCK）
            let settable_mask = OpenFlags::O_APPEND | OpenFlags::O_NONBLOCK;
            // 保留不可修改的位，合并新标志
            let new_flags_val = (current_flags.bits() & !settable_mask.bits())
                | (arg as u32& settable_mask.bits());
            let new_flags = OpenFlags::from_bits_truncate(new_flags_val);

            // 应用新的状态标志
            file.flags=new_flags;
           0
           
        }
        // 其他命令（如文件锁 F_GETLK/F_SETLK）可后续实现
        _ => {
            // 不支持的命令
            EINVAL as isize
        }
    }
}

// 假设 TaskInner/文件表有如下辅助方法：
// impl TaskInner {
//     // 根据 fd 获取文件对象
//     fn get_file(&self, fd: usize) -> Option<Arc<dyn File>> { ... }
//     // 从指定起点分配新的 fd
//     fn alloc_fd_from(&mut self, min_fd: usize) -> Option<usize> { ... }
//     // fd_table: Vec<Option<Arc<dyn File>>>
//     // fd_flags: Vec<usize> // 存储 FD_CLOEXEC 标志
// }

// 假设 File trait 定义：
// pub trait File: Send + Sync {
//     // 获取当前状态标志
//     fn get_status_flags(&self) -> Result<OpenFlags, SomeError>;
//     // 设置状态标志
//     fn set_status_flags(&self, flags: OpenFlags) -> Result<(), SomeError>;
// }

// 假设 OpenFlags 定义（来自 bitflags）：
// bitflags! {
//     pub struct OpenFlags: usize {
//         const O_RDONLY   = 0;
//         const O_WRONLY   = 1;
//         const O_RDWR     = 2;
//         const O_APPEND   = 1024;
//         const O_NONBLOCK = 2048;
//         // …其他标志…
//     }
// }
