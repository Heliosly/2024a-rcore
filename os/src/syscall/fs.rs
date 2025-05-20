//! File and filesystem-related syscalls

use core::slice;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec:: Vec;
use alloc::vec;
use linux_raw_sys::general::{SEEK_CUR, SEEK_END, SEEK_SET};


use crate::config::{MAX_KERNEL_RW_BUFFER_SIZE, PATH_MAX, UIO_MAXIOV};
use crate::fs::{ find_inode, open_file, File,  Kstat,  OpenFlags };
use crate::mm::{get_target_ref, put_data, translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_process, current_task, current_token};
use crate::utils::error::{SysErrNo, SyscallRet};
use crate::utils::normalize_and_join_path;

use super::flags::{ FstatatFlags, IoVec, AT_FDCWD, FD_CLOEXEC, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};


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
            if !file.any().writable().await.map_err(|_| SysErrNo::EIO)? {
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
            if !file.any().readable().await.map_err(|_| SysErrNo::EIO)? {
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
    copy_from_user_array, copy_from_user_bytes, copy_to_user_bytes // 我们基于 copy_from_user_bytes 实现的
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


// --- 依赖的辅助函数 (占位符，你需要提供实际实现) ---
// mod crate::task {
//     pub mod current {
//         use alloc::sync::Arc;
//         use super::{ProcessControlBlock, Task};
//         pub async fn current_process() -> Arc<ProcessControlBlock> { unimplemented!() }
//         // pub fn current_task_token() -> usize { unimplemented!() } // 或者通过 pcb.memory_set.lock().await.token()
//     }
//     // ... ProcessControlBlock, Task, TaskRef, SysErrNo ...
// }
// mod crate::fs {
//     pub enum SeekWhence { SET, CUR, END }
//     #[derive(Debug)] pub struct YourFsError;
//     pub trait FileDescriptor: Send + Sync { // Send + Sync 如果要在 async 中安全共享
//         fn seek(&self, offset: isize, whence: SeekWhence) -> Result<usize, YourFsError>;
//         fn read(&self, buf: &mut [u8]) -> Result<usize, YourFsError>; // 同步方法
//         fn write(&self, buf: &[u8]) -> Result<usize, YourFsError>;  // 同步方法
//         fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, YourFsError>;
//         fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize, YourFsError>;
//         // 如果你的文件操作也是 async, 那么这些方法签名会是 async fn ... -> Result<...>
//         // 并且在调用处需要 .await
//     }
//     impl Clone for Box<dyn FileDescriptor> { /* ... */ } // 如果 FileDescriptor 是 dyn Trait
// }
// mod crate::mm {
//     #[derive(Debug, Copy, Clone, PartialEq, Eq)] pub struct VirtAddr(usize);
//     impl VirtAddr {
//         pub fn from(v: usize) -> Self { Self(v) }
//         pub fn as_usize(&self) -> usize { self.0 }
//     }
//     impl core::ops::Add<usize> for VirtAddr { type Output = Self; fn add(self, rhs: usize) -> Self::Output { Self(self.0.saturating_add(rhs)) } }

//     #[derive(Debug)] pub enum TranslateRefError { TranslationFailed(VirtAddr), DataCrossesPageBoundary, UnexpectedEofOrFault, InternalBufferOverflow, LengthOverflow }
//     pub mod user_mem {
//         use super::*;
//         pub unsafe fn get_target_ref<'a, T>(_token: usize, _ptr: *const T) -> Result<&'a T, TranslateRefError> { unimplemented!() }
//         pub unsafe fn copy_from_user_bytes(_token: usize, _kernel_dest: &mut [u8], _user_src: VirtAddr, _len: usize) -> Result<(), TranslateRefError> { unimplemented!() }
//         pub unsafe fn copy_to_user_bytes(_token: usize, _user_dest: VirtAddr, _kernel_src: &[u8], _len: usize) -> Result<(), TranslateRefError> { unimplemented!() }
//         pub unsafe fn copy_from_user_array<T: Copy>(_token: usize, _user_src_ptr: *const T, _count: usize) -> Result<Vec<T>, TranslateRefError> { unimplemented!() }
//     }
// }
// mod crate::syscall {
//     #[derive(Debug, Copy, Clone, PartialEq, Eq)]
//     pub enum SysErrNo { EINVAL, EBADF, EFAULT, EIO, /* ... */ }
//     impl SysErrNo { pub fn from_fs_error(_err: crate::fs::YourFsError) -> Self { SysErrNo::EIO } }
// }
// mod libc {
//     pub const EINVAL: i32 = 22; pub const EBADF: i32 = 9; pub const EFAULT: i32 = 14;
//     pub const EIO: i32 = 5; pub const UIO_MAXIOV: usize = 1024;
//     pub const SEEK_SET: i32 = 0; pub const SEEK_CUR: i32 = 1; pub const SEEK_END: i32 = 2;
// }