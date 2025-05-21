//! File and filesystem-related syscalls

use crate::signal::SigSet;
use crate::timer::TimeVal;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec:: Vec;
use alloc::vec;

use crate::config::{FD_SETSIZE, MAX_KERNEL_RW_BUFFER_SIZE, PATH_MAX, UIO_MAXIOV};
use crate::fs::{ find_inode, open_file, File, FileDescriptor, Kstat, OpenFlags, PollEvents, PollFd, PollFuture, PollRequest };
use crate::mm::{ put_data, translated_byte_buffer, translated_str, UserBuffer};
use crate::task::sleeplist::sleep_until;
use crate::task::{current_process, current_task, current_token};
use crate::timer::current_time;
use crate::utils::error::{SysErrNo, SyscallRet};
use crate::utils::normalize_and_join_path;

use super::flags::{ FstatatFlags, IoVec, UserTimeSpec, AT_FDCWD, FD_CLOEXEC, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};


pub async fn sys_write(fd: usize, buf: *const u8, len: usize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_write,fd:{}", current_task().get_pid(), fd);
    let token = current_token().await;
    let proc = current_process();
    let fd_table = proc.fd_table.lock().await;

    // 1. 检查 fd 是否越界
    if fd >= fd_table.len() {
        return Err(SysErrNo::EBADF);
    }

    match &fd_table[fd] {
        Some(file) => {
            // 2. 检查是否可写
            if !file.any().writable().map_err(|_| SysErrNo::EIO)? {
                return Err(SysErrNo::EACCES);
            }
            let file = file.clone();
            // 3. 执行写操作
            let bytes = file
                .any()
                .write(UserBuffer::new(translated_byte_buffer(token, buf, len)))
                .await
                .map_err(|_| SysErrNo::EIO)?;
            Ok(bytes )
        }
        None => Err(SysErrNo::EBADF),
    }
}

pub async fn sys_read(fd: usize, buf: *const u8, len: usize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_read,fd:{}", current_task().get_pid(), fd);
    let token = current_token().await;
    let proc = current_process();
    let fd_table = proc.fd_table.lock().await;

    // 1. 检查 fd 是否越界
    if fd >= fd_table.len() {
        return Err(SysErrNo::EBADF);
    }

    match &fd_table[fd] {
        Some(file) => {
            // 2. 检查是否可读
            if !file.any().readable().map_err(|_| SysErrNo::EIO)? {
                return Err(SysErrNo::EACCES);
            }
            let file = file.clone();
            // 3. 执行读操作
            let bytes = file
                .any()
                .read(UserBuffer::new(translated_byte_buffer(token, buf, len)))
                .await
                .map_err(|_| SysErrNo::EIO)?;
            Ok(bytes)
        }
        None => Err(SysErrNo::EBADF),
    }
}
pub async  fn sys_openat(dirfd: isize, path_ptr: *const u8, flags_u32: u32, mode: u32) -> SyscallRet {
    trace!("kernel:pid[{}] sys_open", current_task().get_pid());

    // 1. 获取当前进程
    let proc = current_process();
    let token = proc.memory_set.lock().await.token();

    // 2. 从用户空间读取路径字符串（*const u8 指针）
    let path = translated_str(token, path_ptr);

    // 如果路径是空字符串，返回“没有此文件”错误
    if path.is_empty() {
        return Err(SysErrNo::ENOENT);
    }

    // 如果路径太长，返回“路径名太长”错误
    if path.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }

    // 3. 将 flags_u32 转换成 OpenFlags 结构体（失败就返回“参数无效”）
    let open_flags = OpenFlags::from_bits(flags_u32).ok_or(SysErrNo::EINVAL)?;

    // 4. 决定路径的基准目录（用来拼接最终的绝对路径）
    let base_abs_path: String;

    if path.starts_with('/') {
        // 如果路径是绝对路径（以 / 开头），就忽略 dirfd
        base_abs_path = "/".to_string(); // 根目录作为起点
    } else if dirfd == AT_FDCWD {
        // 如果 dirfd 是 AT_FDCWD，表示相对于当前工作目录
        base_abs_path = proc.cwd.lock().await.clone();
    } else {
        // 否则路径是相对于 dirfd 指定的目录
        let dir_file = proc.get_file(dirfd as usize).await?; // 获取 dirfd 对应的文件对象
        if !dir_file.file.file()?.is_dir() {
            return Err(SysErrNo::ENOTDIR); // 如果不是目录，返回“不是目录”的错误
        }
        base_abs_path = dir_file.file.file()?.get_path(); // 获取这个目录的路径
    }

    // 5. 拼接成最终的绝对路径
    let final_abs_path = normalize_and_join_path(&base_abs_path, &path)?;

    // 路径最终仍然为空且不是 . 或 ""，视为非法
    if final_abs_path.is_empty() && path != "." && path != "" {
        return Err(SysErrNo::ENOENT);
    }

    // 6. 调用 VFS 层的 open_file 函数打开文件
    // open_file 会处理 O_CREATE、O_EXCL、O_TRUNC 等 flag，还会检查权限
    let file_class_instance = open_file(&final_abs_path, open_flags, mode)?;

    // （可选）基于 flags 进行进一步检查
    // 比如 O_DIRECTORY 要求文件是目录，O_NOFOLLOW 不允许跟随符号链接等
    // 通常这些检查最好在 open_file 里做

    // 7. 为打开的文件分配一个新的文件描述符 fd
    let new_fd = proc.alloc_fd().await;
    proc.fd_table.lock().await.get(new_fd).replace(&Some(file_class_instance));

    // 返回新分配的 fd
    Ok(new_fd )
}


/// 关闭一个文件描述符
/// fd: 要关闭的文件描述符
/// 返回: 成功时为 Ok(0), 失败时为 Err(SysErrNo)
pub async  fn sys_close(fd: usize) -> SyscallRet {
    // trace!("kernel:pid[{}] sys_close for fd {}", current_task().get_pid(), fd);
    let proc = current_process();
    let mut fd_table = proc.fd_table.lock().await; // 获取文件描述符表的锁

    // 1. 检查 fd 是否有效
    if fd >= fd_table.len() || fd_table[fd].is_none() {
        return Err(SysErrNo::EBADF); // 无效的文件描述符
    }

    // 2. 从表中移除 FileClass (通过 Option::take)
    // 当 Arc<FileClass> 的最后一个引用被移除时 (如果 FileClass 是 Arc'd),
    // 它的 drop 方法会被调用，触发 VFS 层的清理。
    let _removed_file = fd_table[fd].take(); // Option::take 将 Some(v) 变为 None 并返回 v

    // _removed_file (一个 Option<Arc<FileClass>>) 在这里超出作用域并被 drop。
    // 如果这是 Arc 的最后一个引用, FileClass 的 Drop trait (如果实现) 将被调用。

    Ok(0) 
}


/// YOUR JOB: Implement fstat.
pub async  fn sys_fstat(fd: usize, st: *mut Kstat) -> SyscallRet {
    trace!("kernel:pid[{}] sys_fstat ", current_task().get_pid());
    let proc= current_process();
    let token = proc.memory_set.lock().await.token();

    // if (kst as isize) <= 0 || if_bad_address(kst as usize) {
    //     return Err(SysErrNo::EFAULT);
    // }

    // debug!(
    //     "[sys_fstat] fd is {:?}, kst_addr is {:#x}",
    //     fd, kst as usize
    // );

    if fd >= proc.fd_len().await || proc.get_file(fd).await.is_err() {
        return Err(SysErrNo::EBADF);
    }
    let file = proc.get_file(fd).await?.any();
   put_data(token, st, file.fstat())? ;
    Ok(0)
         
    
 
    
}


pub   fn sys_ioctl(_fd: usize, _cmd: usize, _arg: usize) -> SyscallRet{
    // 伪实现
   Ok(0)
}

pub async  fn sys_fstatat(
    dirfd: isize,
    path_ptr: *const u8,
    kst: *mut Kstat,
    flags: usize,
) -> SyscallRet {
    let token = current_token().await;

    // 从用户指针翻译出路径字符串
    let path = translated_str(token, path_ptr);

    let proc = current_process();

    // 解析 flags，非法时返回 EINVAL
    let flags = FstatatFlags::from_bits(flags)
        .ok_or(SysErrNo::EINVAL)?;

    // 1. 处理 AT_EMPTY_PATH：允许空路径时对 dirfd 本身 stat
    if flags.contains(FstatatFlags::EMPTY_PATH) && path.is_empty() {
        if dirfd == AT_FDCWD {
            return Err(SysErrNo::EINVAL);
        }
        // 校验并获取 dirfd 对应的文件句柄
        let file = proc.get_file(dirfd as usize).await?;
        let inode = file.file().expect("fd should refer to inode-backed file");
        // 将元数据写回用户缓冲区
        put_data(token, kst, inode.fstat())? ;
        return Ok(0);
    }

    // 2. 计算基准路径
    let base_path = if path.starts_with('/') {
        // 绝对路径
        String::new()
    } else if dirfd == AT_FDCWD {
        // 相对于当前工作目录
        proc.cwd.lock().await.clone()
    } else {
        // 相对于 dirfd 指向的目录
        let file = proc.get_file(dirfd as usize).await?;
        let inode = file.file().expect("fd should refer to inode-backed file");
        inode.get_path()
    };
    let full_path = normalize_and_join_path(base_path.as_str(), path.as_str())?;
    // 3. 根据 AT_SYMLINK_NOFOLLOW 决定符号链接处理
    let mut open_flags = OpenFlags::O_RDONLY;
    if flags.contains(FstatatFlags::SYMLINK_NO_FOLLOW) {
        open_flags |= OpenFlags::O_ASK_SYMLINK;
    }

    // 4. 在 VFS 中查找 inode，并写回 Kstat
    let inode = find_inode(&full_path, open_flags)?;
    put_data(token, kst, inode.fstat())?;

    Ok(0)
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(_old_name: *const u8, _new_name: *const u8) -> SyscallRet {
    trace!("kernel:pid[{}] sys_linkat NOT IMPLEMENTED", current_task().get_pid());
    todo!()
    // let token = current_token();
    // let op=translated_str(token, _old_name);
    // let np=translated_str(token, _new_name);
    // if op==np{
    //     return -1;
    // }
    // unsafe { UMAP2.insert(np,op.clone()) };
    
    // if unsafe { ITOS.contains_key(&op) }{
      
    //     unsafe {
    //        ITOS.get_mut(&op).unwrap().link+=1;
          
    //     }
    //     0
    // }
    // else{
    //     -1
    // }
    
   

  
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(_name: *const u8) -> SyscallRet {
    trace!("kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED", current_task().get_pid());
    todo!();
    // let token = current_token();
    // let name=translated_str(token, _name);
    // //println!("suc1");
    // //println!("unlink:np=?{}",name);
    // if unsafe { UMAP2.contains_key(&name) }{
    //      let op = unsafe { UMAP2 [&name].clone()};
    //      //println!("unlink:op=?{}",op);
    //      unsafe { UMAP2.remove(&name) };
    //      if unsafe { ITOS.contains_key(&op) }{
    //         unsafe {
    //            let c=ITOS.get_mut(&op).unwrap();
    //            //println!("suc1");
    //            if c.link>0{
          
    //             c.link-=1;
    //            }
    //            else {
    //             let fd= UMAP1.get(&op).unwrap() ;
    //         current_process().fd_table.lock().await.get(*fd).unwrap().clone().unwrap().file().unwrap().clear();
    //            }
    //         }
    //     }    
    
   
    //  return 0;
    // }
    // else 
    // {
    //        let  c= unsafe { ITOS .get_mut(&name).unwrap()};
    //        if c.link>0{
          
    //         c.link-=1;
    //        }
    //        else{
    //          let fd=unsafe { UMAP1.get(&name).unwrap() };
    //         current_process().fd_table.lock().await.get(*fd).unwrap().clone().unwrap().file().unwrap().clear();

    //        }
          
           
               
    // }
//    Ok(0)
}



pub async  fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> Result<usize, SysErrNo> {
    let proc = current_process();

    match cmd {
        // —— 复制描述符 —— //
        F_DUPFD | F_DUPFD_CLOEXEC => {
            // 1) 验证旧 fd
            let file = proc.get_file(fd).await?;

            // 2) 从 arg 开始分配新 fd
            let min_fd = arg;
            let new_fd = proc
                .alloc_fd_from(min_fd).await
                .ok_or(SysErrNo::EMFILE)?;

            // 3) 如果 new_fd 超出表长，则扩容
            if new_fd >= proc.fd_len().await {
                proc.fd_table.lock().await.resize_with(new_fd + 1, || None);
            }

            // 4) 克隆并存回 table
            {
                let mut table = proc.fd_table.lock().await;
                table[new_fd] = Some(file.clone());
            }

            // 5) 根据命令设置/清除 CLOEXEC
            let mut new_file = proc.get_file(new_fd).await?;
            if cmd == F_DUPFD_CLOEXEC {
                new_file.set_cloexec();
            } else {
                new_file.unset_cloexec();
            }

            Ok(new_fd)
        }

        // —— 读 FD 标志 —— //
        F_GETFD => {
            let file = proc.get_file(fd).await?;
            // 只返回 FD_CLOEXEC 位
            Ok(file.cloexec() as usize)
        }

        // —— 写 FD 标志 —— //
        F_SETFD => {
            let mut file = proc.get_file(fd).await?;
            // 仅取 arg 的最低位作为 FD_CLOEXEC
            if (arg & FD_CLOEXEC) != 0 {
                file.set_cloexec();
            } else {
                file.unset_cloexec();
            }
            Ok(0)
        }

        // —— 读文件状态标志 —— //
        F_GETFL => {
            let file = proc.get_file(fd).await?;
            // flags 包含 O_APPEND、O_NONBLOCK 等
            Ok(file.flags.bits() as usize)
        }

        // —— 写文件状态标志 —— //
        F_SETFL => {
            let mut file = proc.get_file(fd).await?;
            // 只允许修改 O_APPEND 和 O_NONBLOCK
            let settable = OpenFlags::O_APPEND | OpenFlags::O_NONBLOCK;
            let current = file.flags.bits();
            let new_bits = (current & !settable.bits()) | ((arg as u32) & settable.bits());
            file.flags = OpenFlags::from_bits_truncate(new_bits);
            Ok(0)
        }

        // —— 其他命令暂不支持 —— //
        _ => Err(SysErrNo::EINVAL),
    }
}













// 假设的类型和常量 (你需要根据你的项目调整)
use crate::mm::{VirtAddr, TranslateRefError, PageTable}; // 你的内存管理类型

// 导入我们新定义的内存复制函数 (假设它们在 mm 模块或一个新模块 user_mem)
use crate::mm::page_table::{
     // 你提供的，但主要用于单页、类型化数据
    copy_from_user_array, copy_from_user_bytes, copy_from_user_exact, copy_to_user_bytes // 我们基于 copy_from_user_bytes 实现的
    // 如果 TranslateRefError 需要扩展，确保也导入或定义
};


// --- 1. Syscall::Lseek (lseek) ---
// lseek 不直接访问用户数据缓冲区，主要操作 fd 和偏移量，所以改动不大
pub async fn sys_lseek(fd: usize, offset: isize, whence: usize) -> SyscallRet {
    // log::trace!("sys_lseek(fd: {}, offset: {}, whence: {})", fd, offset, whence);
    let pcb_arc = current_process(); // 假设 current_process 是 async
    let fd_table_guard = pcb_arc.fd_table.lock(); // 假设是同步锁

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }

    let file_descriptor = match fd_table_guard.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(), // 注意 FileDescriptor 的共享/生命周期
        None => return Err(SysErrNo::EBADF),
    };
    // drop(fd_table_guard); // 可以在这里释放，如果 file_descriptor.seek 不依赖它

   

    // FileDescriptor 的 seek 方法本身可能是同步的
    // 如果是 async fn seek(&self, ...) -> ... 则需要 .await
    Ok(file_descriptor.file()?.lseek(offset, whence) ?)
}

// --- 2. Syscall::Pread64 (pread64) ---
pub async fn sys_pread64(fd: usize, user_buf_ptr: *mut u8, count: usize, offset: usize) -> SyscallRet {
    // log::trace!("sys_pread64(fd: {}, buf_ptr: {:p}, count: {}, offset: {})", fd, user_buf_ptr, count, offset);
    if count == 0 {
        return Ok(0);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock();

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(),
        None => return Err(SysErrNo::EBADF),
    };
    drop(fd_table_guard); // 释放 fd_table 锁

    if user_buf_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 在内核中创建一个临时缓冲区来接收数据
    // FIXME: 对于大 count，避免大 Vec 分配。考虑使用页对齐的缓冲区池或直接操作用户页。
    let mut kernel_buffer = vec![0u8; count.min(MAX_KERNEL_RW_BUFFER_SIZE)]; // 限制单次内核缓冲区大小
    let actual_count_to_read_this_pass = kernel_buffer.len();

    // FileDescriptor 的 read_at 方法本身可能是同步的
   let bytes_read =file_descriptor.file()?.read_at(offset, &mut kernel_buffer[0..actual_count_to_read_this_pass]) ?;

       
            if bytes_read == 0 { // EOF
                return Ok(0);
            }
            
           
               Ok( unsafe { copy_to_user_bytes(
                    token,
                    VirtAddr::from(user_buf_ptr as usize),
                    &kernel_buffer[0..bytes_read],
                ) }?
            )
            
            
            
        }
       
    



pub async fn sys_pwrite64(fd: usize, user_buf_ptr: *const u8, count: usize, offset: usize) -> SyscallRet {
    // log::trace!("sys_pwrite64(fd: {}, buf_ptr: {:p}, count: {}, offset: {})", fd, user_buf_ptr, count, offset);
    if count == 0 {
        return Ok(0);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock();

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(),
        None => return Err(SysErrNo::EBADF),
    };
    drop(fd_table_guard);

    if user_buf_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 从用户空间安全地复制数据到内核缓冲区
    // FIXME: 考虑分块处理大 count
    let mut kernel_buffer = vec![0u8; count.min(MAX_KERNEL_RW_BUFFER_SIZE)];
    let actual_count_to_write_this_pass = kernel_buffer.len();

     unsafe {
        copy_from_user_bytes(
            token,
            &mut kernel_buffer[0..actual_count_to_write_this_pass],
            VirtAddr::from(user_buf_ptr as usize),
            actual_count_to_write_this_pass,
        )
    }?;
      
           file_descriptor.file()?.write_at(offset, &kernel_buffer[0..actual_count_to_write_this_pass])
       
}



// --- 4. Syscall::Readv (readv) ---
/// --- 4. Syscall::Readv (readv) ---
pub async fn sys_readv(fd:  usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {
    if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
        return Err(SysErrNo::EINVAL);
    }
    let iov_count = iovcnt as usize;

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock();

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(),
        None => return Err(SysErrNo::EBADF),
    };
    drop(fd_table_guard);

    // 1. 从用户空间安全地复制 iovec 数组到内核
    let kernel_iovs: Vec<IoVec> = match unsafe {
        copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
    } {
        Ok(iovs) => iovs,
        Err(_) => return Err(SysErrNo::EFAULT),
    };

    // 2. 校验每个 iovec 条目
    let mut total_len_to_read_request: usize = 0;
    for iov_entry in &kernel_iovs {
        if iov_entry.base.is_null() && iov_entry.len > 0 {
            return Err(SysErrNo::EFAULT);
        }
        total_len_to_read_request = total_len_to_read_request.saturating_add(iov_entry.len);
    }
    if total_len_to_read_request == 0 {
        return Ok(0);
    }

    // 3. 循环读取每个 iovec 指定的缓冲区
    let mut total_bytes_read: usize = 0;
    // 内核中转缓冲区
    let mut temp_kernel_chunk = Box::pin(vec![0u8; MAX_KERNEL_RW_BUFFER_SIZE.min(total_len_to_read_request)]);

    let temp_chunk_len = temp_kernel_chunk.len();
    for iov_entry in &kernel_iovs {
        if iov_entry.len == 0 {
            continue;
        }

        let mut bytes_read_for_this_iov: usize = 0;
        let mut user_iov_offset: usize = 0;

        while bytes_read_for_this_iov < iov_entry.len {
          
            let len_this_pass = (iov_entry.len - bytes_read_for_this_iov)
                .min(temp_chunk_len);
            if len_this_pass == 0 {
                break;
            }

             let buf_slice = &mut temp_kernel_chunk[..len_this_pass];
            let read_result = {
                file_descriptor
                    .file()?
                    .read(UserBuffer { buffers: vec![buf_slice] })
                    .await
            };

            match read_result {
                Ok(0) => {
                    // EOF
                    return Ok(total_bytes_read);
                }
                Ok(bytes_read_into_chunk) => {
                    // 复制到用户空间
                    let user_dest_ptr = unsafe { iov_entry.base.add(user_iov_offset) };
                    match unsafe {
                        copy_to_user_bytes(
                            token,
                            VirtAddr::from(user_dest_ptr as usize),
                            &temp_kernel_chunk[..bytes_read_into_chunk],
                        )
                    } {
                        Ok(_) => {
                            total_bytes_read += bytes_read_into_chunk;
                            bytes_read_for_this_iov += bytes_read_into_chunk;
                            user_iov_offset += bytes_read_into_chunk;

                            if bytes_read_into_chunk < len_this_pass {
                                // 读到的少于请求，提前返回
                                return Ok(total_bytes_read);
                            }
                        }
                        Err(_) => {
                            if total_bytes_read > 0 {
                                return Ok(total_bytes_read);
                            } else {
                                return Err(SysErrNo::EFAULT);
                            }
                        }
                    }
                }
                Err(fs_error) => {
                    if total_bytes_read > 0 {
                        return Ok(total_bytes_read);
                    } else {
                        return Err(fs_error);
                    }
                }
            }
        }
    }

    Ok(total_bytes_read)
}



// --- 5. Syscall::Writev (writev) ---
pub async fn sys_writev(fd:     usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {
    // log::trace!("sys_writev(fd: {}, iov_ptr: {:p}, iovcnt: {})", fd, iov_user_ptr, iovcnt);

    if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
        return Err(SysErrNo::EINVAL);
    }
    let iov_count = iovcnt as usize;

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock().await;

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(),
        None => return Err(SysErrNo::EBADF),
    };
    drop(fd_table_guard);

    let kernel_iovs: Vec<IoVec> = match unsafe {
        copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
    } {
        Ok(iovs) => iovs,
        Err(_) => return Err(SysErrNo::EFAULT),
    };

    let mut total_len_to_write_request: usize = 0;
    for iov_entry in kernel_iovs.iter() {
        if iov_entry.base.is_null() && iov_entry.len > 0 {
            return Err(SysErrNo::EFAULT);
        }
        total_len_to_write_request = total_len_to_write_request.saturating_add(iov_entry.len);
    }
    if total_len_to_write_request == 0 {
        return Ok(0);
    }

    let mut total_bytes_written: usize = 0;
    let mut temp_kernel_chunk = vec![0u8; MAX_KERNEL_RW_BUFFER_SIZE.min(total_len_to_write_request)];

        let chunk_len= temp_kernel_chunk.len();
    for iov_entry in kernel_iovs.iter() { // kernel_iovs 是内核的副本
        if iov_entry.len == 0 {
            continue;
        }

        let mut bytes_written_for_this_iov: usize = 0;
        let mut user_iov_offset: usize = 0;
        while bytes_written_for_this_iov < iov_entry.len {
            let len_this_pass = (iov_entry.len - bytes_written_for_this_iov)
                                .min(chunk_len);
            if len_this_pass == 0 { break; }
       
            let slice = &mut temp_kernel_chunk[0..len_this_pass];
            // 从用户 iovec 的当前部分复制数据到内核中转缓冲区
            let user_src_ptr = unsafe { (iov_entry.base as *const u8).add(user_iov_offset) };
            match unsafe {
                copy_from_user_bytes(
                    token,
                    slice,
                    VirtAddr::from(user_src_ptr as usize),
                    len_this_pass,
                )
            } {
                Ok(()) => {
                    // 从内核中转缓冲区写入文件
                    match file_descriptor.write(UserBuffer{buffers:vec![ slice]}
                ).await{
                        Ok(0) => { // 写入0字节但没有错误，通常意味着不能再写入 (例如管道另一端关闭)
                            return Ok(total_bytes_written );
                        }
                        Ok(bytes_written_from_chunk) => {
                            total_bytes_written += bytes_written_from_chunk;
                            bytes_written_for_this_iov += bytes_written_from_chunk;
                            user_iov_offset += bytes_written_from_chunk;

                            if bytes_written_from_chunk < len_this_pass {
                                // 文件提前结束写入 (例如磁盘满)
                                return Ok(total_bytes_written );
                            }
                        }
                        Err(fs_error) => {
                            if total_bytes_written > 0 { return Ok(total_bytes_written ); }
                            else { return Err(fs_error); }
                        }
                    }
                }
                Err(translate_error) => {
                    // log::warn!("sys_writev: copy_from_user failed for iov: {:?}", translate_error);
                    if total_bytes_written > 0 { return Ok(total_bytes_written); }
                    else { return Err(SysErrNo::EFAULT); }
                }
            }
        }
    }
    Ok(total_bytes_written )
}

// --- sys_poll 系统调用实现 ---
pub async fn sys_poll(user_fds_ptr: *mut PollFd, nfds: usize, timeout_ms: i32) -> SyscallRet {
    if nfds == 0 {
        if timeout_ms > 0 {
            let deadline = current_time().add_milliseconds(timeout_ms as usize);
            sleep_until(Some(deadline)).await; // sleep_until 现在接受 Option
            return Ok(0);
        } else if timeout_ms == 0 {
            return Ok(0);
        } else { // timeout_ms < 0 (无限等待)
            // 对于 nfds = 0 且无限等待，任务应该挂起直到被信号中断。
            // 我们可以 await 一个永不超时的 SleepFuture。
            // 当它被信号或其他方式唤醒时，SleepFuture 会 Ready。
            // POSIX poll 此时返回 -EINTR。
            sleep_until(None).await; // 等待，直到被唤醒（例如信号）
            return Err(SysErrNo::EINTR); // 假设被信号中断
        }
    }

    if nfds > FD_SETSIZE as usize { // 使用你配置的 FD_SETSIZE
        return Err(SysErrNo::EINVAL);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();

    let user_pollfds_kernel_copy: Vec<PollFd> = if user_fds_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    } else {
        match unsafe { copy_from_user_array::<PollFd>(token, user_fds_ptr, nfds) } {
            Ok(fds) => fds,
            Err(_) => return Err(SysErrNo::EFAULT),
        }
    };

    let mut parsed_requests: Vec<PollRequest> = Vec::with_capacity(nfds);
    let fd_table_guard = pcb_arc.fd_table.lock().await;

    for (idx, user_pfd) in user_pollfds_kernel_copy.iter().enumerate() {
        let mut fd_arc_opt: Option< FileDescriptor> = None;
        let mut effective_events = user_pfd.events;
        if user_pfd.fd >= 0 {
            if let Some(fd_instance_opt_in_table) = fd_table_guard.get(user_pfd.fd as usize) {
                if let Some(fd_instance_arc_in_table) = fd_instance_opt_in_table.as_ref() {
                    // 假设 fd_table 存储的是 Arc<dyn FileDescriptor>
                    fd_arc_opt = Some(fd_instance_arc_in_table.clone());
                }
            }
        } else {
            effective_events = PollEvents::empty();
        }
        parsed_requests.push(PollRequest ::new(
            idx,
            user_pfd.fd,
            fd_arc_opt,
            effective_events,
        ));
    }
    drop(fd_table_guard);

    // 计算传递给 PollFuture::new 的 Option<TimeVal>
    let poll_future_timeout: Option<TimeVal> = if timeout_ms == 0 {
        // 不等待，但 PollFuture 仍然需要被 poll 一次以检查即时就绪的 FD
        // 我们可以传递 Some(current_time())，这样 SleepFuture 会立即超时。
        Some(current_time())
    } else if timeout_ms > 0 {
        Some(current_time().add_milliseconds(timeout_ms as usize))
    } else { // timeout_ms < 0 (无限等待)
        None // 传递 None 给 PollFuture，它会创建一个 deadline 为 None 的 SleepFuture
    };

    let poll_future = PollFuture::new(
        token,
        parsed_requests,
        user_fds_ptr,
        nfds,
        poll_future_timeout, // 传递计算好的 Option<TimeVal>
    );

    poll_future.await
}


/// ppoll 系统调用实现
/// fds_ptr: 用户空间 struct pollfd 数组的指针 (usize)
/// nfds: pollfd 数组的元素数量 (usize)
/// tmo_p: 用户空间 struct timespec 指针，或0 (NULL) 表示无限等待 (usize)
/// sigmask_ptr: 用户空间 sigset_t 指针，或0 (NULL) 表示不改变信号掩码 (usize)
/// 返回：SyscallRet
pub async fn sys_ppoll(
    fds_user_ptr: *mut PollFd, // 直接用 *mut PollFd 更类型安全
    nfds: usize,
    tmo_user_ptr: *const UserTimeSpec, // 指向用户空间的 timespec
    sigmask_user_ptr: *const SigSet,   // 指向用户空间的 sigset_t
) -> SyscallRet {
    // log::trace!("sys_ppoll(fds_ptr: {:p}, nfds: {}, tmo_p: {:p}, sigmask_ptr: {:p})",
    //             fds_user_ptr, nfds, tmo_user_ptr, sigmask_user_ptr);

    // 1. 处理 nfds = 0 的情况 (与 poll 类似，但要注意信号掩码的设置和恢复)
    if nfds == 0 {
        let mut old_sigmask_to_restore: Option<SigSet> = None;
        if !sigmask_user_ptr.is_null() {
            // 原子地设置新信号掩码，并保存旧掩码
            // 这需要一个内部函数，我们称之为 sys_sigprocmask_internal
            // 它不直接是系统调用，而是内核内部操作信号掩码的逻辑
            // match sys_sigprocmask_internal(libc::SIG_SETMASK, sigmask_user_ptr, Some(&mut old_mask_for_restore)) {
            //     Ok(old_mask) => old_sigmask_to_restore = Some(old_mask),
            //     Err(e) => return Err(e), // 复制或设置掩码失败
            // }
            // 简化：假设我们有一个函数可以原子地交换掩码
            // old_sigmask_to_restore = Some(swap_current_thread_sigmask_from_user(token, sigmask_user_ptr)?);
            // 这里需要更底层的实现。我们将使用一个 RAII Guard 来确保恢复。
            // 或者在 defer 块中恢复。
            // 为了简化，我们假设我们有一个临时的内部函数
            let pcb_arc_temp = current_process(); // 获取 pcb 以访问 token 和任务状态
            let token_temp = pcb_arc_temp.memory_set.lock().await.token();
            match set_temp_sigmask_from_user(token_temp, sigmask_user_ptr).await {
                Ok(Some(old_mask)) => old_sigmask_to_restore = Some(old_mask),
                Ok(None) => {} // sigmask_user_ptr is null
                Err(e) => return Err(e),
            }
        }

        // defer 恢复信号掩码 (RAII Guard 是更好的方式)
        // struct SigmaskRestorer(Option<SigSet>);
        // impl Drop for SigmaskRestorer { fn drop(&mut self) { if let Some(mask) = self.0 { restore_old_sigmask(mask); }}}
        // let _restorer = SigmaskRestorer(old_sigmask_to_restore);

        let result: SyscallRet;
        if tmo_user_ptr.is_null() { // 无限等待
            sleep_until(None).await;
            result = Err(SysErrNo::EINTR); // 假设被信号中断
        } else {
            let pcb_arc_temp = current_process(); // 获取 pcb 以访问 token 和任务状态
            let token_temp = pcb_arc_temp.memory_set.lock().await.token();
            // 从用户空间复制 timespec
            let timeout_spec = match unsafe { copy_from_user_exact::<UserTimeSpec>(token_temp, tmo_user_ptr) } {
                Ok(ts) => ts,
                Err(_) => {
                    if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
                    return Err(SysErrNo::EFAULT);
                }
            };
            if  timeout_spec.tv_nsec >= 1_000_000_000 {
                if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask); }
                return Err(SysErrNo::EINVAL); // 无效的 timespec
            }
            if timeout_spec.tv_sec == 0 && timeout_spec.tv_nsec == 0 { // 零超时
                result = Ok(0);
            } else {
                let deadline = current_time().add_timespec(&timeout_spec); // 假设 TimeVal 有此方法
                sleep_until(Some(deadline)).await;
                result = Ok(0); // 超时后返回0
            }
        }

        if let Some(old_mask) = old_sigmask_to_restore {
            restore_sigmask_internal(old_mask).await; // 恢复原始信号掩码
        }
        return result;
    }


    // 2. 检查 nfds 上限
    if nfds > FD_SETSIZE as usize {
        return Err(SysErrNo::EINVAL);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();

    // 3. 原子地设置新的信号掩码 (如果提供了 sigmask_user_ptr) 并保存旧的
    let mut old_sigmask_to_restore: Option<SigSet> = None;
    if !sigmask_user_ptr.is_null() {
        // 使用一个内部函数来原子地设置掩码并返回旧掩码
        // 这个函数需要访问当前线程的信号状态
        match set_temp_sigmask_from_user(token, sigmask_user_ptr).await {
            Ok(Some(old_mask)) => old_sigmask_to_restore = Some(old_mask),
            Ok(None) => {} // sigmask_user_ptr is null, no change
            Err(e) => return Err(e),
        }
    }

    // 使用 RAII Guard 或 defer 模式确保信号掩码在函数返回前被恢复
    // struct SigmaskGuard(Option<SigSet>);
    // impl Drop for SigmaskGuard { fn drop(&mut self) { if let Some(mask) = self.0.take() { restore_sigmask_internal(mask); }}}
    // let _sigmask_guard = SigmaskGuard(old_sigmask_to_restore.clone()); // Clone Option<SigSet>

    // 4. 从用户空间复制 PollFd 数组 (与 sys_poll 相同)
    let user_pollfds_kernel_copy: Vec<PollFd> = if fds_user_ptr.is_null() {
        if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
        return Err(SysErrNo::EFAULT);
    } else {
        match unsafe { copy_from_user_array::<PollFd>(token, fds_user_ptr, nfds) } {
            Ok(fds) => fds,
            Err(_) => {
                if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
                return Err(SysErrNo::EFAULT);
            }
        }
    };

    // 5. 构建 parsed_requests (与 sys_poll 相同)
    let mut parsed_requests: Vec<PollRequest> = Vec::with_capacity(nfds);
    let fd_table_guard = pcb_arc.fd_table.lock().await; // 假设 fd_table 锁是 async

    for (idx, user_pfd) in user_pollfds_kernel_copy.iter().enumerate() {
        let mut fd_arc_opt: Option< FileDescriptor> = None;
        let mut effective_events = user_pfd.events;
        if user_pfd.fd >= 0 {
            if let Some(fd_instance_opt_in_table) = fd_table_guard.get(user_pfd.fd as usize) {
                if let Some(fd_instance_arc_in_table) = fd_instance_opt_in_table.as_ref() {
                    fd_arc_opt = Some(fd_instance_arc_in_table.clone());
                }
            }
        } else {
            effective_events = PollEvents::empty();
        }
        parsed_requests.push(PollRequest {
            fd_index: idx,
            original_user_fd: user_pfd.fd,
            file_descriptor: fd_arc_opt,
            requested_events: effective_events,
        });
    }
    drop(fd_table_guard);

    // 6. 计算超时 deadline (从 timespec)
    let poll_future_timeout_deadline: Option<TimeVal> = if tmo_user_ptr.is_null() {
        None // 无限等待
    } else {
        let timeout_spec = match unsafe { copy_from_user_exact::<UserTimeSpec>(token, tmo_user_ptr) } {
            Ok(ts) => ts,
            Err(_) => {
                if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask); }
                return Err(SysErrNo::EFAULT);
            }
        };
        if  timeout_spec.tv_nsec >= 1_000_000_000 {
            if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask); }
            return Err(SysErrNo::EINVAL);
        }
        if timeout_spec.tv_sec == 0 && timeout_spec.tv_nsec == 0 { // 零超时
            Some(current_time()) // 立即超时
        } else {
            Some(current_time().add_timespec(&timeout_spec)) // 转换为绝对时间
        }
    };

    // 7. 创建并等待 PollFuture
    let poll_future = PollFuture::new(
        token,
        parsed_requests,
        fds_user_ptr,
        nfds,
        poll_future_timeout_deadline,
    );

    let result = poll_future.await; // SyscallRet

    // 8. 恢复原始信号掩码 (在所有路径上都应执行)
    if let Some(old_mask) = old_sigmask_to_restore {
        restore_sigmask_internal(old_mask).await;
    }

    result
}


// --- 内部辅助函数，用于原子地设置和恢复信号掩码 ---
// 这些函数需要访问当前线程的 ThreadSignalState.sigmask
// 它们不是系统调用，而是内核内部的辅助。

/// 尝试从用户空间设置临时信号掩码，并返回旧的掩码。
/// 如果 sigmask_user_ptr 为 NULL，则不改变掩码并返回 Ok(None)。
async fn set_temp_sigmask_from_user(token: usize, sigmask_user_ptr: *const SigSet) -> Result<Option<SigSet>, SysErrNo> {
    if sigmask_user_ptr.is_null() {
        return Ok(None);
    }

    let new_sigmask_from_user = match unsafe { copy_from_user_exact::<SigSet>(token, sigmask_user_ptr) } {
        Ok(s) => s,
        Err(_) => return Err(SysErrNo::EFAULT),
    };

    let current_task_arc = current_task(); // 需要能获取当前任务的 Arc<Task>
    let mut thread_signal_state = current_task_arc.signal_state.lock().await; // 假设 Task 有 signal_thread_state

    let old_mask = thread_signal_state.sigmask;
    
    let mut new_mask_to_set = new_sigmask_from_user;
    // SIGKILL 和 SIGSTOP 不能被阻塞
    new_mask_to_set.remove(crate::signal::Signal::SIGKILL); // 假设 Signal 枚举路径
    new_mask_to_set.remove(crate::signal::Signal::SIGSTOP);

    thread_signal_state.sigmask = new_mask_to_set;
    Ok(Some(old_mask))
}

/// 恢复旧的信号掩码。
async fn restore_sigmask_internal(old_mask: SigSet) {
    let current_task_arc = current_task();
    let mut thread_signal_state = current_task_arc.signal_state.lock().await;
    thread_signal_state.sigmask = old_mask;
}

