//! File and filesystem-related syscalls
use core::mem;

use alloc::collections::btree_map::BTreeMap;
use alloc::string::{String, ToString};

pub struct Ino{
    link:u32,
    ino:u64,
}
use crate::fs::{ find_inode, open_file, File, Kstat, OSInode, OpenFlags, Stat, StatMode};
use crate::mm::{translated_byte_buffer, translated_refmut, translated_str, UserBuffer};
use crate::task::{current_process, current_task, current_token};
use crate::utils::error::SysErrNo;

use super::flags::{FstatatFlags, AT_FDCWD};
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
        if !file.writable().await.unwrap() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))).await.unwrap() as isize
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
        if !file.readable().await.unwrap() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        // trace!("kernel: sys_read .. file.read");
       let res= file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))).await.unwrap() as isize;

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
    match open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap(), 0o777) {
        Ok(inode) => {
            let fd = proc.alloc_fd();
            let mut fd_table = proc.fd_table.lock();
           fd_table[fd] = Some(inode.file().unwrap());
    
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

pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> isize{
    
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
        let inode = file.as_any()
            .downcast_ref::<OSInode>()
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
        let inode = file.as_any()
            .downcast_ref::<OSInode>()
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
            current_process().fd_table.lock().get(*fd).unwrap().clone().unwrap().clear();
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
            current_process().fd_table.lock().get(*fd).unwrap().clone().unwrap().clear();

           }
          
           
               
    }
   0
}
