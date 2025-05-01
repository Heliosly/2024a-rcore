//! File and filesystem-related syscalls
use core::mem;

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;

pub struct Ino{
    link:u32,
    ino:u64,
}
use crate::fs::{ OpenFlags, Stat, StatMode};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};
pub static mut UMAP:BTreeMap<usize,String>=BTreeMap::new();
pub static mut UMAP1:BTreeMap<String,usize>=BTreeMap::new();
pub static mut UMAP2:BTreeMap<String,String>=BTreeMap::new();
pub static mut ITOS:BTreeMap<String,Ino>=BTreeMap::new();

pub static mut IDX:u64=1;
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
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
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        unsafe { UMAP.insert(fd, path.clone()) };
        unsafe { UMAP1.insert(path.clone(),fd) };
        unsafe{
            
            if !ITOS.contains_key(&path.clone()){
                ITOS.insert(path, Ino{
                    link:1,
                    ino:IDX,
                    
               });
               
               IDX+=1;
            }
            

        }
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();

    unsafe { UMAP1.remove(&UMAP[&fd]) };
    unsafe { UMAP.remove(&fd) };
  
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(_fd: usize, _st: *mut Stat) -> isize {
    trace!(
        "kernel:pid[{}] sys_fstat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let task=current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if _fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[_fd].is_none() {
        return -1;
    }
    drop(inner);
   
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
      let token=current_user_token();
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

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(_old_name: *const u8, _new_name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_linkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token=current_user_token();
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
    trace!(
        "kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token=current_user_token();
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
             let bind=current_task().unwrap();
             let inner=bind.inner_exclusive_access().fd_table[*fd].clone().unwrap();
             inner.clear();
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
             let bind=current_task().unwrap();
             let inner=bind.inner_exclusive_access().fd_table[*fd].clone().unwrap();
             inner.clear();

           }
          
           
               
    }
   0
}
