//! File and filesystem-related syscalls

use crate::fs::dev::open_device_file;
use crate::fs::mount::MNT_TABLE;
use crate::fs::pipe::make_pipe;
use crate::fs::vfs::VfsManager;
use crate::signal::SigSet;
use crate::syscall::flags::{FaccessatMode, MlockallFlags};
use crate::timer::{TimeVal,UserTimeSpec};
use crate::utils::string::{get_abs_path, get_parent_path_and_filename, is_abs_path};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec:: Vec;
use alloc::vec;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_SET};
const NONE_MODE:u32=0;
use crate::config::{FD_SETSIZE, MAX_FD_NUM, MAX_KERNEL_RW_BUFFER_SIZE, PATH_MAX, SENDFILE_KERNEL_BUFFER_SIZE, UIO_MAXIOV};
use crate::fs::{ find_inode, open_file, remove_inode_idx, File, FileClass, FileDescriptor, Kstat, OpenFlags, PollEvents, PollFd, PollFuture, PollRequest, Statfs };
use crate::mm::{ put_data, translated_byte_buffer, translated_refmut, translated_str, UserBuffer};
use crate::task::sleeplist::sleep_until;
use crate::task::{current_process, current_task, current_token};
use crate::timer::current_time;
use crate::utils::error::{SysErrNo, SyscallRet};
use crate::utils::normalize_and_join_path;

use super::flags::{ FstatatFlags, IoVec,  AT_FDCWD, FD_CLOEXEC, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};
use super::process;


pub async fn sys_write(fd: usize, buf: *const u8, len: usize) -> SyscallRet {
    info!("kernel:pid[{}] sys_write,fd:{}", current_task().get_pid(), fd);
    let token = current_token().await;
    let proc = current_process();
    let fd_table = proc.fd_table.lock().await;

    // 1. 检查 fd 是否越界
    if fd >= fd_table.len() {
        return Err(SysErrNo::EBADF);
    }
    
    match &fd_table.table[fd] {
        Some(file) => {
            // 2. 检查是否可写
            if !file.any().writable()? {
                return Err(SysErrNo::EACCES);
            }
            let file = file.clone();
            // 3. 执行写操作
            let bytes = file
                .any()
                .write(UserBuffer::new(translated_byte_buffer(token, buf, len)))
                .await
                ?;
            Ok(bytes )
        }
        None => Err(SysErrNo::EBADF),
    }
}

pub async fn sys_read(fd: usize, buf: *const u8, len: usize) -> SyscallRet {
    info!("[sys_read],fd:{}",  fd);
    let token = current_token().await;
    let proc = current_process();
    let fd_table = proc.fd_table.lock().await;

    // 1. 检查 fd 是否越界
    if fd >= fd_table.len() {
        return Err(SysErrNo::EBADF);
    }

    match &fd_table.table[fd] {
        Some(file) => {
            // 2. 检查是否可读
            if !file.any().readable()? {
                return Err(SysErrNo::EACCES);
            }
            let file = file.clone();
            // 3. 执行读操作
            let bytes = file
                .any()
                .read(UserBuffer::new(translated_byte_buffer(token, buf, len)))
                .await
               ?;
            Ok(bytes)
        }
        None => Err(SysErrNo::EBADF),
    }
}
pub async  fn sys_openat(dirfd: i32, path_ptr: *const u8, flags_u32: u32, mode: u32) -> SyscallRet {
    info!(
        "[sys_openat] dirfd = {}, path_ptr = {:p}, flags = {:#x}, mode = {:#o}",
        dirfd,
        path_ptr,
        flags_u32,
        mode,
    );
  
    // 1. 获取当前进程
    
    let proc = current_process();
    let token = proc.memory_set.lock().await.token();
    proc.manual_alloc_type_for_lazy(path_ptr).await?;
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
    } else if dirfd == AT_FDCWD  {
        // 如果 dirfd 是 AT_FDCWD，表示相对于当前工作目录
        base_abs_path = proc.cwd.lock().await.clone();
    } else {
        // 否则路径是相对于 dirfd 指定的目录
        
        let dir_file = proc.fd_table.lock().await.get_file(dirfd as usize)?; // 获取 dirfd 对应的文件对象
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
    let new_fd = proc.fd_table.lock().await.alloc_fd()?;
    proc.fd_table.lock().await.table[new_fd] = Some(file_class_instance);
    // 返回新分配的 fd
    Ok(new_fd )
}


/// 关闭一个文件描述符
/// fd: 要关闭的文件描述符
/// 返回: 成功时为 Ok(0), 失败时为 Err(SysErrNo)
pub async  fn sys_close(fd: i32) -> SyscallRet {
    trace!(" [sys_close] for fd {}", fd);
    let proc = current_process();
    let mut fd_table = proc.fd_table.lock().await; // 获取文件描述符表的锁
    let fd_usize= fd as usize;
    // 1. 检查 fd 是否有效
    if fd_usize >= fd_table.len() || fd_table.table[fd_usize].is_none() {
        return Err(SysErrNo::EBADF); // 无效的文件描述符
    }

    // 2. 从表中移除 FileClass (通过 Option::take)
    // 当 Arc<FileClass> 的最后一个引用被移除时 (如果 FileClass 是 Arc'd),
    // 它的 drop 方法会被调用，触发 VFS 层的清理。
    let _removed_file = fd_table.table[fd_usize].take(); // Option::take 将 Some(v) 变为 None 并返回 v

    // _removed_file (一个 Option<Arc<FileClass>>) 在这里超出作用域并被 drop。
    // 如果这是 Arc 的最后一个引用, FileClass 的 Drop trait (如果实现) 将被调用。

    Ok(0) 
}


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
    let table  = proc.fd_table.lock().await;
    if fd >= table.len() || table.get_file(fd).is_err() {
        return Err(SysErrNo::EBADF);
    }
    let file = table.get_file(fd)?.any();
    put_data(token, st, file.fstat())? ;
    Ok(0)

    
 
    
}


pub   fn sys_ioctl(_fd: usize, _cmd: usize, _arg: usize) -> SyscallRet{
    // 伪实现
    warn!("[sys_ioctl]");
   Ok(0)
}

pub async  fn sys_fstatat(
    dirfd: i32,
    path_ptr: *const u8,
    kst: *mut Kstat,
    flags: usize,
) -> SyscallRet {

    let proc = current_process();
    let mut ms=proc.memory_set.lock().await;
    // 从用户指针翻译出路径字符串
    let token  =ms.token();
    let path  =ms.safe_translated_str( path_ptr).await;

    trace!("[sys_fstatat] dirfd: {}, path: {:?}, kst: {:?}, flags: {}", dirfd, path, kst, flags);

    // 解析 flags，非法时返回 EINVAL
    let flags = FstatatFlags::from_bits(flags)
        .ok_or(SysErrNo::EINVAL)?;

    // 1. 处理 AT_EMPTY_PATH：允许空路径时对 dirfd 本身 stat
    if flags.contains(FstatatFlags::EMPTY_PATH) && path.is_empty() {

        if dirfd == AT_FDCWD {
            return Err(SysErrNo::EINVAL);
        }
        // 校验并获取 dirfd 对应的文件句柄

        let file = proc.fd_table.lock().await.get_file(dirfd as usize)?;
        // 将元数据写回用户缓冲区
        trace!("fstat");
        put_data(token, kst, file.any().fstat())? ;
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
        let file = proc.fd_table.lock().await.get_file(dirfd as usize)?;
        let inode = file.file().expect("abs file should use AT_EMPTY_PATH flags");
        inode.get_path()
    };
    let full_path = normalize_and_join_path(base_path.as_str(), path.as_str())?;
    // 3. 根据 AT_SYMLINK_NOFOLLOW 决定符号链接处理
    let mut open_flags = OpenFlags::O_RDONLY;
    if flags.contains(FstatatFlags::SYMLINK_NO_FOLLOW) {
        open_flags |= OpenFlags::O_ASK_SYMLINK;
    }

    // 4. 在 VFS 中查找 inode，并写回 Kstat
    let inode = open_file(&full_path, open_flags,0)?;
    put_data(token, kst, inode.fstat())?;

    Ok(0)
}





pub async  fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> Result<usize, SysErrNo> {
    trace!("[sys_fcntl],fd:{},cmd:{},arg:{}",fd,cmd,arg);
    let proc = current_process();

    let mut file_table = proc.fd_table.lock().await;
    match cmd {
        // —— 复制描述符 —— //
        F_DUPFD | F_DUPFD_CLOEXEC => {
            // 1) 验证旧 fd
            let file = file_table.get_file(fd)?;

            // 2) 从 arg 开始分配新 fd
            let min_fd = arg;
            let new_fd = file_table
                .alloc_fd_from(min_fd)
                .ok_or(SysErrNo::EMFILE)?;

            // 3) 如果 new_fd 超出表长，则扩容
            if new_fd >= file_table.len() {
                file_table.table.resize_with(new_fd + 1, || None);
            }

            // 4) 克隆并存回 table
            
                file_table.table[new_fd] = Some(file.clone());
           

            // 5) 根据命令设置/清除 CLOEXEC
            let mut new_file = file_table.get_file(new_fd)?;
            if cmd == F_DUPFD_CLOEXEC {
                new_file.set_cloexec();
            } else {
                new_file.unset_cloexec();
            }

            Ok(new_fd)
        }

        // —— 读 FD 标志 —— //
        F_GETFD => {
            let file = file_table.get_file(fd)?;
            // 只返回 FD_CLOEXEC 位
            Ok(file.cloexec() as usize)
        }

        // —— 写 FD 标志 —— //
        F_SETFD => {
            let mut file = file_table.get_file(fd)?;
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
            let file = file_table.get_file(fd)?;
            // flags 包含 O_APPEND、O_NONBLOCK 等
            Ok(file.flags.bits() as usize)
        }

        // —— 写文件状态标志 —— //
        F_SETFL => {
            let mut file = file_table.get_file(fd)?;
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













use crate::mm::{VirtAddr, TranslateError, PageTable}; 

// 导入我们新定义的内存复制函数 (假设它们在 mm 模块或一个新模块 user_mem)
use crate::mm::page_table::{
     // 你提供的，但主要用于单页、类型化数据
    copy_from_user_array, copy_from_user_bytes, copy_from_user_exact, copy_to_user_bytes, copy_to_user_bytes_exact // 我们基于 copy_from_user_bytes 实现的
    // 如果 TranslateRefError 需要扩展，确保也导入或定义
};


// --- 1. Syscall::Lseek (lseek) ---
// lseek 不直接访问用户数据缓冲区，主要操作 fd 和偏移量
pub async fn sys_lseek(fd: usize, offset: isize, whence: u32) -> SyscallRet {
    log::trace!("sys_lseek(fd: {}, offset: {}, whence: {})", fd, offset, whence);
    let pcb_arc = current_process(); // 假设 current_process 是 async
    let fd_table_guard = pcb_arc.fd_table.lock().await; 

    if fd  >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }

    let file_descriptor =  fd_table_guard.get_file(fd)?;
    // drop(fd_table_guard); // 可以在这里释放，如果 file_descriptor.seek 不依赖它

   

    // FileDescriptor 的 seek 方法本身可能是同步的
    // 如果是 async fn seek(&self, ...) -> ... 则需要 .await
    Ok(file_descriptor.any().lseek(offset, whence) ?)
}

// --- 2. Syscall::Pread64 (pread64) ---
pub async fn sys_pread64(fd: i32, user_buf_ptr: *mut u8, count: usize, offset: usize) -> SyscallRet {
    log::trace!("sys_pread64(fd: {}, buf_ptr: {:p}, count: {}, offset: {})", fd, user_buf_ptr, count, offset);
    if count == 0 {
        return Ok(0);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock().await;

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.table.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
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
    log::trace!("sys_pwrite64(fd: {}, buf_ptr: {:p}, count: {}, offset: {})", fd, user_buf_ptr, count, offset);
    if count == 0 {
        return Ok(0);
    }

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let fd_table_guard = pcb_arc.fd_table.lock();

    if fd < 0 || fd as usize >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF);
    }
    let file_descriptor = match fd_table_guard.table.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
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



// // --- 4. Syscall::Readv (readv) ---
// /// --- 4. Syscall::Readv (readv) ---
// pub async fn sys_readv(fd:  usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {

// trace!("[sys_readv]");
//     if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
//         return Err(SysErrNo::EINVAL);
//     }
//     let iov_count = iovcnt as usize;

//     let pcb_arc = current_process();
//     let token = pcb_arc.memory_set.lock().await.token();
//     let fd_table_guard = pcb_arc.fd_table.lock();

//     if fd < 0 || fd as usize >= fd_table_guard.len() {
//         return Err(SysErrNo::EBADF);
//     }
//     let file_descriptor = match fd_table_guard.table.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
//         Some(f) => f.clone(),
//         None => return Err(SysErrNo::EBADF),
//     };
//     drop(fd_table_guard);

//     // 1. 从用户空间安全地复制 iovec 数组到内核
//     let kernel_iovs: Vec<IoVec> = match unsafe {
//         copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
//     } {
//         Ok(iovs) => iovs,
//         Err(_) => return Err(SysErrNo::EFAULT),
//     };

//     // 2. 校验每个 iovec 条目
//     let mut total_len_to_read_request: usize = 0;
//     for iov_entry in &kernel_iovs {
//         if iov_entry.base.is_null() && iov_entry.len > 0 {
//             return Err(SysErrNo::EFAULT);
//         }
//         total_len_to_read_request = total_len_to_read_request.saturating_add(iov_entry.len);
//     }
//     if total_len_to_read_request == 0 {
//         return Ok(0);
//     }

//     // 3. 循环读取每个 iovec 指定的缓冲区
//     let mut total_bytes_read: usize = 0;
//     // 内核中转缓冲区
//     let mut temp_kernel_chunk = Box::pin(vec![0u8; MAX_KERNEL_RW_BUFFER_SIZE.min(total_len_to_read_request)]);

//     let temp_chunk_len = temp_kernel_chunk.len();
//     for iov_entry in &kernel_iovs {
//         if iov_entry.len == 0 {
//             continue;
//         }

        
//         let mut bytes_read_for_this_iov: usize = 0;
//         let mut user_iov_offset: usize = 0;

//         while bytes_read_for_this_iov < iov_entry.len {
          
//             let len_this_pass = (iov_entry.len - bytes_read_for_this_iov)
//                 .min(temp_chunk_len);
//             if len_this_pass == 0 {
//                 break;
//             }

//              let buf_slice = &mut temp_kernel_chunk[..len_this_pass];
//             let read_result = {
//                 file_descriptor
//                     .file()?
//                     .read(UserBuffer { buffers: vec![buf_slice] })
//                     .await
//             };

//             match read_result {
//                 Ok(0) => {
//                     // EOF
//                     return Ok(total_bytes_read);
//                 }
//                 Ok(bytes_read_into_chunk) => {
//                     // 复制到用户空间
//                     let user_dest_ptr = unsafe { iov_entry.base.add(user_iov_offset) };
//                     match unsafe {
//                         copy_to_user_bytes(
//                             token,
//                             VirtAddr::from(user_dest_ptr as usize),
//                             &temp_kernel_chunk[..bytes_read_into_chunk],
//                         )
//                     } {
//                         Ok(_) => {
//                             total_bytes_read += bytes_read_into_chunk;
//                             bytes_read_for_this_iov += bytes_read_into_chunk;
//                             user_iov_offset += bytes_read_into_chunk;

//                             if bytes_read_into_chunk < len_this_pass {
//                                 // 读到的少于请求，提前返回
//                                 return Ok(total_bytes_read);
//                             }
//                         }
//                         Err(_) => {
//                             if total_bytes_read > 0 {
//                                 return Ok(total_bytes_read);
//                             } else {
//                                 return Err(SysErrNo::EFAULT);
//                             }
//                         }
//                     }
//                 }
//                 Err(fs_error) => {
//                     if total_bytes_read > 0 {
//                         return Ok(total_bytes_read);
//                     } else {
//                         return Err(fs_error);
//                     }
//                 }
//             }
//         }
//     }

//     Ok(total_bytes_read)
// }

pub async fn sys_readv(fd: usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {
    trace!("[sys_readv]"); // 日志跟踪

    // 校验 iovcnt 是否合法
    if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
        return Err(SysErrNo::EINVAL); // 无效参数
    }
    let iov_count = iovcnt as usize; // 转换为 usize

    // 获取当前进程控制块 (PCB) 和内存集 token
    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token(); // token 用于后续内存操作的上下文
    let fd_table_guard = pcb_arc.fd_table.lock().await; // 获取文件描述符表锁

    // 校验文件描述符 fd 是否有效
    // fd 是 usize 类型，所以它不可能是负数，只需检查是否超出范围
    if fd >= fd_table_guard.len() {
        return Err(SysErrNo::EBADF); // 坏的文件描述符
    }
    // 获取文件描述符对应的文件对象
    let file_descriptor = match fd_table_guard.table.get(fd).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(), // 克隆 Arc<dyn FileLike>
        None => return Err(SysErrNo::EBADF), // 文件描述符在该位置为空
    };
    drop(fd_table_guard); // 及时释放文件描述符表的锁

    // 1. 从用户空间安全地复制 iovec 数组到内核
    //    这一步仍然是必要的，因为我们需要 iovec 结构本身在内核中。
    //    `translated_byte_buffer` 用于数据缓冲区，而不是 iovec 描述符数组。
    let kernel_iovs: Vec<IoVec> = match unsafe {
        copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
    } {
        Ok(iovs) => iovs,
        Err(e) => return Err(e.into()), 
    };

    // 2. 校验每个 iovec 条目并计算总的潜在读取长度
    let mut total_len_to_read_request: usize = 0;
    for iov_entry in &kernel_iovs {
        if iov_entry.base.is_null() && iov_entry.len > 0 {
            return Err(SysErrNo::EFAULT); // iov_entry.base 为空指针但长度大于0，无效地址
        }
        // 还可以检查 iov_entry.len 是否过大，或 total_len_to_read_request 是否溢出
        // 例如: if iov_entry.len > some_sane_max_buffer_per_iov { return Err(SysErrNo::EINVAL); }
        total_len_to_read_request = total_len_to_read_request.saturating_add(iov_entry.len);
    }

    if total_len_to_read_request == 0 {
        return Ok(0); // 总读取请求长度为0，直接返回成功，读取字节数为0
    }

    // 3. 循环处理每个 iovec 条目，直接将数据读入用户缓冲区
    let mut total_bytes_read: usize = 0; // 记录总共读取的字节数

    for iov_entry in &kernel_iovs {
        if iov_entry.len == 0 {
            continue; // 跳过长度为0的 iovec 条目
        }

        // 关键步骤：为当前的 iovec 条目获取用户内存的直接可变引用
        // `translated_byte_buffer` 应该返回 Vec<&'static mut [u8]>
        // 这里的 'static 生命周期是一个 Rust 的技巧，用于 FFI 或不安全代码，
        // 内核保证这些切片在它们的使用期间是有效的（即，在文件I/O操作期间）。
        let user_memory_slices: Vec<&'static mut [u8]> = unsafe {
            // 需要确保 `translated_byte_buffer` 正确处理 `iov_entry.base` 指针和 `iov_entry.len`。
            // 它应该执行所有必要的安全检查（例如，地址有效性，用户权限，页表映射等）。
            // 如果 `translated_byte_buffer` 可能失败（例如，坏地址），它应该返回 Result。
            // 鉴于其签名 `-> Vec<&'static mut [u8]>`，它要么成功，要么 panic/UB (这在内核中通常是不可接受的)。
            // 一个健壮的内核函数应该返回 Result。我们假设如果它返回了，就是“安全的”。
            // `token` 用于指定在哪个进程的地址空间中翻译这些地址。
            translated_byte_buffer(token, iov_entry.base as *const u8, iov_entry.len)
        };
        
        // 重要检查：`translated_byte_buffer` 返回的切片总长度必须等于 `iov_entry.len`。
        // 如果返回的长度更少，意味着它无法映射整个用户缓冲区，这是一个错误。
        // (或者 `translated_byte_buffer` 的设计就是返回它能成功映射的部分，但这需要明确约定)
        let actual_mapped_len = user_memory_slices.iter().map(|s| s.len()).sum::<usize>();
        if actual_mapped_len < iov_entry.len {
            // 这意味着 `translated_byte_buffer` 未能提供完整的内存区域。
            // 这是一个 EFAULT 状况。如果之前的 iovecs 已经读取了一些字节，
            // Linux 通常会返回到目前为止读取的字节数。否则，它是 EFAULT。
            if total_bytes_read > 0 {
                return Ok(total_bytes_read);
            } else {
                return Err(SysErrNo::EFAULT); // 无效内存地址
            }
        }
        // 如果请求长度大于0，但没有映射到任何内存片，也是错误
        if user_memory_slices.is_empty() && iov_entry.len > 0 {
             // 类似于上面的情况，对于非零请求无法映射任何内容。
            if total_bytes_read > 0 {
                return Ok(total_bytes_read);
            } else {
                return Err(SysErrNo::EFAULT);
            }
        }


        // 从这些切片创建一个 UserBuffer
        // UserBuffer::new 需要 Vec<&'static mut [u8]>
        let mut user_buffer = UserBuffer::new(user_memory_slices); 

        // 调用文件对象的 read 方法，传入 UserBuffer
        // 文件系统的 read 实现将通过 UserBuffer 直接写入用户内存。
        let read_result =  file_descriptor.any().read(user_buffer).await;
        match read_result {
            Ok(0) => {
                // 文件到达末尾 (EOF)。返回到目前为止总共读取的字节数。
                // 不再尝试读取后续的 iovecs。
                return Ok(total_bytes_read);
            }
            Ok(bytes_read_into_iov) => {
                total_bytes_read += bytes_read_into_iov;

                // 如果文件读取的字节数少于当前 iovec 的容量，
                // 这意味着遇到了 EOF 或部分读取的情况（例如，管道现在没有更多数据了，或者文件本身就这么短）。
                // 在这种情况下，readv 也应该返回，即使还有更多的 iovecs。这是 POSIX 标准行为。
                if bytes_read_into_iov < iov_entry.len {
                    return Ok(total_bytes_read);
                }
                // 如果 bytes_read_into_iov == iov_entry.len，则当前 iovec 已填满，继续处理下一个 iovec。
            }
            Err(fs_error) => {
                // 在读取操作期间发生错误。
                // 如果之前的一些 iovecs 已经成功读取了字节，
                // POSIX 规定 readv 应该返回那个计数。
                // 否则，返回错误。
                if total_bytes_read > 0 {
                    return Ok(total_bytes_read);
                } else {
                    // 你可能需要在这里将 FsError 映射到 SysErrNo
                    // 例如: return Err(map_fs_error_to_syscall_error(fs_error));
                    return Err(fs_error); // 假设 FsError 与 SysErrNo 兼容或可以转换
                }
            }
        }
    }

    // 如果循环正常结束，意味着所有 iovecs 都被处理（可能被填满，或者在最后一个 iovec 中遇到 EOF/短读）
    Ok(total_bytes_read)
}

// // --- 5. Syscall::Writev (writev) ---
// pub async fn sys_writev(fd:     usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {
//     log::trace!("sys_writev(fd: {}, iov_ptr: {:p}, iovcnt: {})", fd, iov_user_ptr, iovcnt);

//     if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
//         return Err(SysErrNo::EINVAL);
//     }
//     let iov_count = iovcnt as usize;

//     let pcb_arc = current_process();
//     let token = pcb_arc.memory_set.lock().await.token();
//     let fd_table_guard = pcb_arc.fd_table.lock().await;

//     if fd < 0 || fd as usize >= fd_table_guard.len() {
//         return Err(SysErrNo::EBADF);
//     }
//     let file_descriptor = match fd_table_guard.table.get(fd as usize).and_then(|opt_fd| opt_fd.as_ref()) {
//         Some(f) => f.clone(),
//         None => return Err(SysErrNo::EBADF),
//     };
//     drop(fd_table_guard);

//     let kernel_iovs: Vec<IoVec> = match unsafe {
//         copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
//     } {
//         Ok(iovs) => iovs,
//         Err(_) => return Err(SysErrNo::EFAULT),
//     };

//     let mut total_len_to_write_request: usize = 0;
//     for iov_entry in kernel_iovs.iter() {
//         if iov_entry.base.is_null() && iov_entry.len > 0 {
//             return Err(SysErrNo::EFAULT);
//         }
//         total_len_to_write_request = total_len_to_write_request.saturating_add(iov_entry.len);
//     }
//     if total_len_to_write_request == 0 {
//         return Ok(0);
//     }

//     let mut total_bytes_written: usize = 0;
//     let mut temp_kernel_chunk = vec![0u8; MAX_KERNEL_RW_BUFFER_SIZE.min(total_len_to_write_request)];

//         let chunk_len= temp_kernel_chunk.len();
//     for iov_entry in kernel_iovs.iter() { // kernel_iovs 是内核的副本
//         if iov_entry.len == 0 {
//             continue;
//         }

//         let mut bytes_written_for_this_iov: usize = 0;
//         let mut user_iov_offset: usize = 0;
//         while bytes_written_for_this_iov < iov_entry.len {
//             let len_this_pass = (iov_entry.len - bytes_written_for_this_iov)
//                                 .min(chunk_len);
//             if len_this_pass == 0 { break; }
       
//             let slice = &mut temp_kernel_chunk[0..len_this_pass];
//             // 从用户 iovec 的当前部分复制数据到内核中转缓冲区
//             let user_src_ptr = unsafe { (iov_entry.base as *const u8).add(user_iov_offset) };
//             match unsafe {
//                 copy_from_user_bytes(
//                     token,
//                     slice,
//                     VirtAddr::from(user_src_ptr as usize),
//                     len_this_pass,
//                 )
//             } {
//                 Ok(()) => {
//                     // 从内核中转缓冲区写入文件
//                     match file_descriptor.write(UserBuffer{buffers:vec![ slice]}
//                 ).await{
//                         Ok(0) => { // 写入0字节但没有错误，通常意味着不能再写入 (例如管道另一端关闭)
//                             return Ok(total_bytes_written );
//                         }
//                         Ok(bytes_written_from_chunk) => {
//                             total_bytes_written += bytes_written_from_chunk;
//                             bytes_written_for_this_iov += bytes_written_from_chunk;
//                             user_iov_offset += bytes_written_from_chunk;

//                             if bytes_written_from_chunk < len_this_pass {
//                                 // 文件提前结束写入 (例如磁盘满)
//                                 return Ok(total_bytes_written );
//                             }
//                         }
//                         Err(fs_error) => {
//                             if total_bytes_written > 0 { return Ok(total_bytes_written ); }
//                             else { return Err(fs_error); }
//                         }
//                     }
//                 }
//                 Err(translate_error) => {
//                     log::warn!("sys_writev: copy_from_user failed for iov: {:?}", translate_error);
//                     if total_bytes_written > 0 { return Ok(total_bytes_written); }
//                     else { return Err(SysErrNo::EFAULT); }
//                 }
//             }
//         }
//     }
//     Ok(total_bytes_written )
// }
// --- 5. Syscall::Writev (writev) ---
pub async fn sys_writev(fd: usize, iov_user_ptr: *const IoVec, iovcnt: i32) -> SyscallRet {
    trace!("[sys_writev] fd: {}, iov_ptr: {:p}, iovcnt: {}", fd, iov_user_ptr, iovcnt);

    // 1. 校验 iovcnt 是否合法
    if iovcnt <= 0 || iovcnt > UIO_MAXIOV as i32 {
        return Err(SysErrNo::EINVAL); // 无效参数
    }
    let iov_count = iovcnt as usize; // 转换为 usize

    // 2. 获取当前进程上下文和文件描述符
    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token(); // 获取内存集 token
    let fd_table_guard = pcb_arc.fd_table.lock().await; // 获取文件描述符表锁 (注意 .await)

    // 校验文件描述符 fd 是否有效
    if fd >= fd_table_guard.len() { // fd 是 usize，所以不可能是负数
        return Err(SysErrNo::EBADF); // 坏的文件描述符
    }
    let file_descriptor = match fd_table_guard.table.get(fd).and_then(|opt_fd| opt_fd.as_ref()) {
        Some(f) => f.clone(), // 克隆 Arc<dyn FileLike>
        None => return Err(SysErrNo::EBADF), // 文件描述符在该位置为空
    };
    drop(fd_table_guard); // 及时释放文件描述符表的锁

    // 3. 从用户空间安全地复制 iovec 数组到内核
    //    这一步仍然是必要的，因为我们需要 iovec 结构本身在内核中。
    let kernel_iovs: Vec<IoVec> = match unsafe {
        copy_from_user_array::<IoVec>(token, iov_user_ptr, iov_count)
    } {
        Ok(iovs) => iovs,
        Err(e) => return Err(e.into()), // copy_from_user_array 应返回合适的 SysErrNo
    };

    // 4. 校验每个 iovec 条目并计算总的潜在写入长度
    let mut total_len_to_write_request: usize = 0;
    for iov_entry in &kernel_iovs {
        if iov_entry.base.is_null() && iov_entry.len > 0 {
            return Err(SysErrNo::EFAULT); // iov_entry.base 为空指针但长度大于0，无效地址
        }
        total_len_to_write_request = total_len_to_write_request.saturating_add(iov_entry.len);
    }

    if total_len_to_write_request == 0 {
        return Ok(0); // 总写入请求长度为0，直接返回成功，写入字节数为0
    }

    // 5. 循环处理每个 iovec 条目，直接从用户缓冲区读取数据并写入文件
    let mut total_bytes_written: usize = 0; // 记录总共写入的字节数

    for iov_entry in &kernel_iovs { // kernel_iovs 是内核中 iovec 描述符的副本
        if iov_entry.len == 0 {
            continue; // 跳过长度为0的 iovec 条目
        }

        // 关键步骤：为当前的 iovec 条目获取用户内存的直接引用
        // 对于 writev，这些用户内存区域是数据源，内核将从中读取。
        // `translated_byte_buffer` 返回 Vec<&'static mut [u8]>。
        // 尽管我们只是从中读取，但 UserBuffer 的构造函数和文件系统的 write 方法
        // 可能期望可变切片（即使它们内部可能只读取）。
        let user_memory_slices: Vec<&'static mut [u8]> = unsafe {
            // `token` 用于指定在哪个进程的地址空间中翻译这些地址。
            // 对于 writev，需要确保这些用户内存是可读的。
            translated_byte_buffer(token, iov_entry.base as *const u8, iov_entry.len)
        };

        // 校验 `translated_byte_buffer` 是否成功映射了整个请求的区域
        let actual_mapped_len = user_memory_slices.iter().map(|s| s.len()).sum::<usize>();
        if actual_mapped_len < iov_entry.len {
            // 未能映射用户请求的全部内存。
            // 如果已经写入了一些字节，则返回已写入的字节数，否则返回 EFAULT。
            if total_bytes_written > 0 {
                return Ok(total_bytes_written);
            } else {
                return Err(SysErrNo::EFAULT); // 无效内存地址
            }
        }
        // 如果请求长度大于0，但没有映射到任何内存片，也是错误
        if user_memory_slices.is_empty() && iov_entry.len > 0 {
            if total_bytes_written > 0 {
                return Ok(total_bytes_written);
            } else {
                return Err(SysErrNo::EFAULT);
            }
        }

        // 从这些用户内存切片创建一个 UserBuffer 作为数据源
        let user_buffer = UserBuffer::new(user_memory_slices);
        // 注意: UserBuffer 内部的 `read` 方法是从 UserBuffer 中读取数据到 Vec<u8>。
        // 而 File::write 方法期望 UserBuffer 作为参数，它会从这个 UserBuffer "读取" 数据并写入文件。
        
        // 调用文件对象的 write 方法，传入 UserBuffer
        let write_result =  file_descriptor.any().write(user_buffer).await ;// 获取底层的 File Trait 对象
            

        match write_result {
            Ok(0) => {
                // 写入0字节但没有错误。这通常意味着不能再写入了
                // (例如，管道的读取端已关闭，或者磁盘已满但驱动程序/文件系统以这种方式报告)。
                // POSIX 要求返回已写入的字节数。
                return Ok(total_bytes_written);
            }
            Ok(bytes_written_from_iov) => {
                total_bytes_written += bytes_written_from_iov;

                // 如果文件系统写入的字节数少于当前 iovec 提供的字节数，
                // 这意味着发生了部分写入（例如，磁盘空间不足，或管道缓冲区满）。
                // `writev` 系统调用应该返回到目前为止已成功写入的总字节数。
                if bytes_written_from_iov < iov_entry.len {
                    return Ok(total_bytes_written);
                }
                // 如果 bytes_written_from_iov == iov_entry.len，则当前 iovec 的数据已全部写入，
                // 继续处理下一个 iovec。
            }
            Err(fs_error) => {
                println!("err:{:#?}",fs_error);
                // 在写入操作期间发生错误。
                // 如果之前的一些 iovecs 已经成功写入了字节，
                // POSIX 规定 writev 应该返回那个计数。
                // 否则，返回错误。
                if total_bytes_written > 0 {
                    return Ok(total_bytes_written);
                } else {
                    // 你可能需要在这里将 FsError 映射到 SysErrNo
                    return Err(fs_error); // 假设 FsError 与 SysErrNo 兼容或可以转换
                }
            }
        }
    }

    // 如果循环正常结束，意味着所有 iovecs 的数据都被尝试写入
    // (可能每个都被完全写入，或者在最后一个 iovec 中发生了部分写入或EOF(对于写来说不常见)的情况然后返回)
    Ok(total_bytes_written)
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

    if nfds > FD_SETSIZE as usize { 
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
            if let Some(fd_instance_opt_in_table) = fd_table_guard.table.get(user_pfd.fd as usize) {
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
    log::info!("[sys_ppoll](fds_ptr: {:p}, nfds: {}, tmo_p: {:p}, sigmask_ptr: {:p})",
                fds_user_ptr, nfds, tmo_user_ptr, sigmask_user_ptr );

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
            result = Err(SysErrNo::ERESTART); // 假设被信号中断
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
                if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
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

    info!("[sys_ppoll] user_pollfds_kernel_copy: {:?}", user_pollfds_kernel_copy);
    // 5. 构建 parsed_requests (与 sys_poll 相同)
    let mut parsed_requests: Vec<PollRequest> = Vec::with_capacity(nfds);
    let fd_table_guard = pcb_arc.fd_table.lock().await;

    for (idx, user_pfd) in user_pollfds_kernel_copy.iter().enumerate() {
        let mut fd_arc_opt: Option< FileDescriptor> = None;
        let mut effective_events = user_pfd.events;
        if user_pfd.fd >= 0 {
            if let Some(fd_instance_opt_in_table) = fd_table_guard.table.get(user_pfd.fd as usize) {
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
                if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
                return Err(SysErrNo::EFAULT);
            }
        };
        if  timeout_spec.tv_nsec >= 1_000_000_000 {
            if let Some(old_mask) = old_sigmask_to_restore { restore_sigmask_internal(old_mask).await; }
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

pub async fn sys_chdir(path: *const u8) -> SyscallRet {
    trace!("[sys_chdir] path_ptr={:p}", path);
   
    let proc = current_process();
    // 获取当前进程的访问令牌
    let token = proc.get_user_token().await;
    let path_str = translated_str(token, path);
// 如果给定的是绝对路径，则直接使用；否则基于当前工作目录拼接
    let new_path = if is_abs_path(&path_str) {
                path_str.clone()
            } else {
                get_abs_path(&proc.cwd.lock().await, &path_str)
            };
   open_file(&new_path.as_str(), OpenFlags::O_RDONLY, 0)?;
       
            
            // 更新进程的 cwd
    proc.set_cwd(new_path).await;
    Ok(0)
       
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

pub async fn sys_getdents64(fd: usize, buf: *const u8, len: usize) -> SyscallRet {
    let proc = current_process();

    debug!(
        "[sys_getdents64] fd is {}, buf addr  is {:x}, len is {}",
        fd, buf as usize, len
    );
    let fd_table = proc.fd_table.lock().await;
    if fd >= fd_table.len()  {
        return Err(SysErrNo::EINVAL);
    }
    let file =  match fd_table.table.get(fd) {
        Some(file_option) => {
            match file_option {
                Some(file) => file.clone(),
                None => return Err(SysErrNo::EBADF),
            }
        }
        None => return Err(SysErrNo::EBADF),
    };
    let inode=file.file()?;
    let mut buffer =
        UserBuffer::new(proc.memory_set.lock().await.
        safe_translated_byte_buffer( buf, len).await);

    let off;
    let check_off = file.lseek(0, SEEK_CUR );
    if let Err(_) = check_off {
        return Ok(0);
    } else {
        off = check_off.unwrap();
    }
    let (de, off) = inode.read_dentry(off, len)?;
    buffer.write(de.as_slice());
    if off!=-1{
        let _ = file.lseek(off , SEEK_SET )?;
    }
    else{
        return Ok(0);
    }
    return Ok(de.len());
}


/// faccessat 系统调用实现
/// dirfd: 目录文件描述符。可以是 AT_FDCWD 表示当前工作目录。
/// path_user_ptr: 指向用户空间路径字符串的指针。
/// mode: 要检查的访问模式 (R_OK, W_OK, X_OK 的组合，或 F_OK)。
/// flags: AT_SYMLINK_NOFOLLOW 或 AT_EACCESS (此实现暂不处理 flags)。
pub async fn sys_faccessat(dirfd: i32, path_user_ptr: *const u8, mode_u32: u32, _flags: usize) -> SyscallRet {
    

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    // 1. 校验参数
    if path_user_ptr.is_null() { // path_user_ptr == 0 或 < 0 都是无效地址
        return Err(SysErrNo::EFAULT);
    }
    // mode_u32 是 u32，不会 < 0。检查是否包含有效位。
    // FaccessatMode::from_bits 会处理无效位，如果它返回 None 或 Err。
    let mode = match FaccessatMode::from_bits(mode_u32) {
        Some(m) => m,
        None => { // 如果 from_bits 返回 None 表示 mode_u32 中有未定义的位
            if mode_u32 == 0 { // mode == 0 (F_OK) 是合法的
                FaccessatMode::empty() // 代表 F_OK
            } else {
                return Err(SysErrNo::EINVAL); // 无效的 mode 位
            }
        }
    };
    // POSIX: If mode is F_OK, permissions are not checked.
    // Your code checks R_OK, W_OK, X_OK based on mode bits.
    // If mode_u32 == 0 (F_OK), then mode.contains(R_OK) etc. will be false.
    // This implicitly handles F_OK by not doing permission checks, only existence (via open).

    // 2. 从用户空间复制路径字符串
    // copy_from_user_str_until_null 需要 token, ptr, max_len
    let path_kernel_str =  translated_str(token, path_user_ptr);
 trace!("[sys_faccessat] dirfd: {}, path_ptr: {}, mode: {}, flags: {}",
                dirfd, path_kernel_str, mode_u32, _flags);
    
  
    // 4. 解析得到最终的、已规范化的绝对路径 abs_path
    let abs_path= pcb_arc.resolve_path_from_fd(  dirfd, &path_kernel_str, false)
    .await?;

    // 4. 检查挂载点只读 (如果请求写权限)
    if mode.contains(FaccessatMode::W_OK) {
       
        if let Some(mount_entry) = crate::fs::mount::MNT_TABLE.lock().get_mount_info_by_dir(&abs_path) {
            let mountflags = mount_entry.flags;
            if (mountflags & 1) != 0 { // 假设 bit 0 是只读标志
                return Err(SysErrNo::EROFS);
            }
        }
    }

    // 5. 获取目标 inode 以检查其存在性和权限
    //    用 O_PATH 或类似的标志来表示只获取元数据而不真正打开。
    //    对于 faccessat，如果文件不存在，应该返回 ENOENT。
    //    faccessat 的权限检查是基于 mode 参数，而不是打开时的权限。

    let target_inode_arc = match find_inode(&abs_path,OpenFlags::O_PATH){ // 假设有异步 lookup_inode
        Ok(inode) => inode,
        Err(SysErrNo::ENOENT) => return Err(SysErrNo::ENOENT), // 文件或路径组件不存在
        Err(e) => return Err(e), // 其他查找错误
    };

    // 如果 mode 是 F_OK (mode_u32 == 0)，并且我们成功 find_inode，说明文件存在。
    if mode_u32 == 0 { // F_OK check
        return Ok(0); // 文件存在，权限检查被跳过
    }

    // 6. 检查父目录的执行权限 (search permission)
    //    这通常是必需的，除非检查的是 AT_FDCWD 下的非斜杠开头路径的顶层组件。
    //    Linux access() 会检查路径中所有目录组件的执行权限。
    if abs_path != "/" { 
        let( parent_path,_) = get_parent_path_and_filename(&abs_path); // 假设有这样的辅助函数

        if !parent_path.is_empty() && parent_path != "/" { // 避免对根目录的父目录（它自己）进行不必要的检查
            let parent_inode_arc =  find_inode(&parent_path.to_string(),OpenFlags::O_DIRECTORY)?;
          
            if !parent_inode_arc.is_dir() {
                return Err(SysErrNo::ENOTDIR); // 路径中的一个组件不是目录
            }
            // 检查父目录的执行权限 (S_IXUSR, S_IXGRP, S_IXOTH)
            // 需要从 pcb_arc 获取 uid, gid
            // let uid=0;//TODO(Heliosly) UID
            // let gid=0;//TODO(Heliosly) GID
            // if !parent_inode_arc.access(uid, gid, FaccessatMode::X_OK) { // 假设 OSInode 有 access 方法
            //     return Err(SysErrNo::EACCES);
            // }
        }
    }


    // 7. 根据请求的 mode 和目标 inode 的权限进行检查
    //    需要从 pcb_arc 获取 uid, gid
    // let uid=0;//TODO(Heliosly) UID
    //         let gid=0;//TODO(Heliosly) GID
    // if !target_inode_arc.access(uid, gid, mode) { // OSInode::access 应该处理 R_OK, W_OK, X_OK
    //     return Err(SysErrNo::EACCES);
    // }

    // 如果所有检查都通过
    Ok(0)


   
}

pub async  fn sys_mkdirat(dirfd: i32, path: *const u8, mode: u32) -> SyscallRet {
    let proc = current_process();
    
    let token = proc.get_user_token().await;
    let path = translated_str(token, path);

   trace!(
        "[sys_mkdirat] dirfd is {},path is {},mode is {}",
        dirfd as isize, path, mode
    );

    if dirfd as isize != -100 && dirfd as usize  >= proc.fd_table.lock().await.len() {
        return Err(SysErrNo::EBADF);
    }

    let abs_path =proc.resolve_path_from_fd(dirfd, &path,false).await?;
    if let Ok(_) = open_file(&abs_path, OpenFlags::O_RDWR, 0) {
        return Err(SysErrNo::EEXIST);
    }
    if let Ok(_) = open_file(
        &abs_path,
        OpenFlags::O_RDWR | OpenFlags::O_CREATE | OpenFlags::O_DIRECTORY,
        mode,
    ) {
        return Ok(0);
    }
    return Err(SysErrNo::ENOENT);
}












// --- sys_dup2 实现 ---
/// dup2 系统调用：复制文件描述符
/// oldfd: 要复制的源文件描述符
/// newfd: 目标文件描述符
/// 返回：成功时为 newfd，失败时为 -错误码
pub async fn sys_dup(oldfd: i32) -> SyscallRet {
    trace!("sys_dup(oldfd: {})", oldfd);

    let pcb_arc = current_process();
    
    // 我们需要同时读和写 fd_table，所以一开始就获取锁
    let mut fd_table = pcb_arc.fd_table.lock().await;

    // 1. 校验 oldfd
    if oldfd < 0 {
        return Err(SysErrNo::EBADF);
    }
    let oldfd_usize = oldfd as usize;
    if oldfd_usize >= fd_table.get_soft_limit(){
        // POSIX 对 oldfd >= RLIMIT_NOFILE 的行为没有明确规定，
        // 但返回 EBADF 是一个合理的选择。
        return Err(SysErrNo::EBADF);
    }

    // 2. 获取源文件描述符的克隆
    let file_to_dup = match fd_table.table.get(oldfd_usize).and_then(|opt| opt.as_ref()) {
        Some(fd_instance) => fd_instance.clone(),
        None => return Err(SysErrNo::EBADF),
    };

    // 3. 分配新的文件描述符
    // 把分配逻辑封装在 FdManage 的方法中更清晰
    let newfd=fd_table.alloc_fd()? ;
     
      fd_table.table[newfd]=Some(file_to_dup);
      fd_table.table[newfd].as_mut().unwrap().unset_cloexec();

      Ok(newfd)
        
    
}

// --- sys_dup3 实现 ---
/// dup3 系统调用：带标志的文件描述符复制
/// oldfd: 要复制的源文件描述符
/// newfd: 目标文件描述符
/// flags: 标志 (目前只支持 O_CLOEXEC)
/// 返回：成功时为 newfd，失败时为 -错误码
pub async fn sys_dup3(oldfd: i32, newfd: i32, flags_u32: u32) -> SyscallRet {
    log::trace!("[sys_dup3](oldfd: {}, newfd: {}, flags: 0x{:x})", oldfd, newfd, flags_u32);

    // 1. 校验 flags (目前只关心 O_CLOEXEC)
    //    如果包含 O_CLOEXEC 之外的其他位，POSIX 要求返回 EINVAL。
    let creation_flags = OpenFlags::from_bits_truncate(flags_u32); // 获取传入的标志
    if !creation_flags.is_empty() && !creation_flags.contains(OpenFlags::FD_CLOEXEC) {
        log::warn!("sys_dup3: Invalid flags 0x{:x} provided. Only O_CLOEXEC is supported.", flags_u32);
        return Err(SysErrNo::EINVAL); // 只允许 O_CLOEXEC 或没有标志
    }

    let pcb_arc = current_process();
    let mut fd_table_guard = pcb_arc.fd_table.lock().await;
   
    // 2. 校验 oldfd 和 newfd 的范围
    if oldfd < 0 || oldfd as usize >= MAX_FD_NUM ||
       newfd < 0 || newfd as usize >= MAX_FD_NUM {
        log::warn!("sys_dup3: oldfd({}) or newfd({}) out of range [0, {}).", oldfd, newfd, MAX_FD_NUM);
        return Err(SysErrNo::EBADF);
    }

    let oldfd_usize = oldfd as usize;
    let newfd_usize = newfd as usize;

    // 3. 如果 oldfd 等于 newfd，dup3 要求返回 EINVAL
    if oldfd == newfd {
        log::warn!("sys_dup3: oldfd({}) is equal to newfd({}). Returning EINVAL.", oldfd, newfd);
        return Err(SysErrNo::EINVAL);
    }
    if oldfd_usize >= fd_table_guard.get_soft_limit(){
        return Err(SysErrNo::EBADF);
    }
    // 4. 检查 oldfd 是否是一个有效的、打开的文件描述符
    let file_to_dup = match fd_table_guard.table.get(oldfd_usize).and_then(|opt| opt.as_ref()) {
        Some(fd_instance) => fd_instance.clone(), // 克隆 FileDescriptor
        None => {
            log::warn!("sys_dup3: oldfd({}) is not a valid open file descriptor.", oldfd);
            return Err(SysErrNo::EBADF);
        }
    };

    // 5. 如果 newfd 已经打开，则先关闭它
    if newfd_usize < fd_table_guard.len() && fd_table_guard.table[newfd_usize].is_some() {
        log::trace!("sys_dup3: Closing already open newfd({}).", newfd);
        fd_table_guard.table[newfd_usize] = None;
    }

    // 6. 确保 fd_table 足够大以容纳 newfd
    if newfd_usize >= fd_table_guard.len() {
        fd_table_guard.table.resize(newfd_usize + 1, None);
    }

    // 7. 将克隆的 FileDescriptor 放入 newfd 的位置，并根据 flags 设置 FD_CLOEXEC
    let mut new_fd_instance = file_to_dup; // file_to_dup 是 FileDescriptor

    // new_fd_instance.flags 是从 oldfd 复制过来的，它可能已经有或没有 FD_CLOEXEC
    // dup3 的 flags 参数是用来 *设置* newfd 的 FD_CLOEXEC 状态，而不是从 oldfd 继承。
    if creation_flags.contains(OpenFlags::FD_CLOEXEC) {
        new_fd_instance.flags.insert(OpenFlags::FD_CLOEXEC);
        log::trace!("sys_dup3: Setting FD_CLOEXEC for newfd({}).", newfd);
    } else {
        new_fd_instance.flags.remove(OpenFlags::FD_CLOEXEC);
        log::trace!("sys_dup3: Clearing FD_CLOEXEC for newfd({}).", newfd);
    }

    fd_table_guard.table[newfd_usize] = Some(new_fd_instance);

    log::debug!("sys_dup3: Duplicated fd {} to {} successfully with flags 0x{:x}.",
                oldfd, newfd, flags_u32);
    Ok(newfd_usize )
}


/// mount 系统调用实现
///
/// # 参数
    /// special: 指向包含设备名称的以 null 结尾的字符串的指针。
    /// dir: 指向包含挂载点路径的以 null 结尾的字符串的指针。
    /// fstype: 指向包含文件系统类型的以 null 结尾的字符串的指针。
    /// flags: 挂载标志。
    /// data_opt: 指向包含特定于文件系统挂载数据的以 null 结尾的字符串的指针。可以为 NULL。
    ///
    /// 返回: 成功时返回 0，失败时返回一个负的错误码。
    ///
    /// 描述:
    /// sys_mount 系统调用将文件系统挂载到指定的挂载点。
    ///
    /// # Errors
    ///
    /// * `EBUSY`: 尝试挂载已经被挂载的设备或目录。
    /// * `EINVAL`: 无效的参数，例如无效的标志或文件系统类型。
    /// * `ENAMETOOLONG`: special 或 dir 超过最大路径长度。
    /// * `ENOENT`: 挂载点目录不存在。
    /// * `EPERM`: 调用者没有足够的权限执行挂载操作。
    /// * `EROFS`: 尝试以只读方式挂载只读文件系统。
    /// * `EXDEV`: 尝试在不支持跨设备挂载的文件系统上挂载。
    ///
    pub async fn sys_mount(
        special_user_ptr: *const u8,
        dir_user_ptr: *const u8,
        fstype_user_ptr: *const u8,
        flags: u32,
        data_user_ptr: *const u8,
    ) -> SyscallRet {
        trace!(
            "[sys_mount] special: {:p}, dir: {:p}, fstype: {:p}, flags: {}, data: {:p}",
            special_user_ptr, dir_user_ptr, fstype_user_ptr, flags, data_user_ptr
        );
    
        // --- 用户空间交互部分 (保持不变) ---
        let pcb_arc = current_process();
        let token = pcb_arc.memory_set.lock().await.token();
        let special = translated_str(token, special_user_ptr);
        let dir = translated_str(token, dir_user_ptr);
        let fstype = translated_str(token, fstype_user_ptr);
        let data_opt: Option<String> = if data_user_ptr.is_null() {
            None
        } else {
            Some(translated_str(token, data_user_ptr))
        };
    
        // --- 核心逻辑部分 (修改) ---
        // 将所有参数委托给 VfsManager::mount 处理。
        // VfsManager 会负责创建驱动实例、检查父目录、更新挂载表等所有工作。
        let result = VfsManager::mount(&special, &dir, &fstype, flags, data_opt);
    
        // 将 VfsManager 的返回结果 (GeneralRet, 即 Result<(), SysErrNo>) 转换为系统调用返回值
        Ok(0) // 成功时，将 Ok(()) 映射为 0
    }

/// umount 系统调用实现
///
/// # 参数
/// * `target_user_ptr`: 指向挂载点路径的用户空间指针。
/// * `flags`: 卸载标志（例如 MNT_FORCE）。
///
/// # 返回值
/// 成功时返回 0，失败时返回负的错误码。
///
/// # 错误
/// * `EINVAL`: 无效的标志。
/// * `EBUSY`: 文件系统繁忙，无法卸载。
/// * `ENOENT`: 目标挂载点不存在。
/// * `EPERM`: 调用者没有权限执行卸载操作。
///
/// # 描述
/// `sys_umount` 函数用于卸载指定路径上的文件系统。`flags` 参数可以用于指定卸载操作的附加选项。
pub async fn sys_umount2(target_user_ptr: *const u8, flags: u32) -> SyscallRet {
    trace!("[sys_umount2] target: {:p}, flags: {}", target_user_ptr, flags);
    
    // --- 用户空间交互部分 (保持不变) ---
    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();
    let target = translated_str(token, target_user_ptr);

    
    let result = VfsManager::umount(&target);

    result.map(|_| 0)
}


pub async  fn sys_unlinkat(dirfd: i32, path: *const u8, _flags: u32) -> SyscallRet {
    // assert!(flags != AT_REMOVEDIR, "not support yet");
    trace!(
        "[sys_unlinkat] dirfd: {}, path: {:p}, flags: {}",
        dirfd, path, _flags
    );
    let proc = current_process();
    let token = proc.get_user_token().await;

    let path = translated_str(token, path);
    let abs_path = proc.resolve_path_from_fd(dirfd, &path,true).await?;
    // 如果是File但尚有对应的fd未关闭,等到close时unlink
    // 如果是符号链接,直接移除
    // 如果是socket, FIFO, or device,移除但现有的fd可继续使用
    let osfile = open_file(&abs_path, OpenFlags::O_ASK_SYMLINK,0 )?.file()?;
   

    
    if  osfile.inner.lock().inode.link_cnt()? == 1 && proc.fd_table.lock().await.find_fd(&path).is_some() {
        osfile.inner.lock().inode.delay();
        remove_inode_idx(&abs_path);
    } else {
        osfile.inner.lock().inode.unlink(&abs_path)?;
        remove_inode_idx(&abs_path);
    }

    Ok(0)
}

pub async fn sys_renameat(
    olddirfd: i32,
    oldpath: *const u8,
    newdirfd: i32,
    newpath: *const u8,
) -> SyscallRet {
    trace!(
        "[sys_renameat] olddirfd: {}, oldpath: {:p}, newdirfd: {}, newpath: {:p}",
        olddirfd, oldpath, newdirfd, newpath
    );

    let proc = current_process();
    let token = proc.get_user_token().await;

    let old_path = translated_str(token, oldpath);
    let new_path = translated_str(token, newpath);

    if old_path.is_empty() || new_path.is_empty() {
        return Err(SysErrNo::ENOENT);
    }
    let old_abs_path = proc.resolve_path_from_fd(olddirfd, &old_path, false).await?;
    let new_abs_path = proc.resolve_path_from_fd(newdirfd, &new_path, false).await?;

    if old_abs_path.len() > PATH_MAX || new_abs_path.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }

    let old_inode = find_inode(&old_abs_path, OpenFlags::O_RDWR)?;
    //let new_inode = find_inode(&new_abs_path, OpenFlags::O_RDWR)?;

    old_inode.rename(&old_abs_path,&new_abs_path)?;

    Ok(0)
}
pub async fn sys_renameat2(
    olddirfd: i32,
    oldpath: *const u8,
    newdirfd: i32,
    newpath: *const u8,
    flags: u32,
) -> SyscallRet{
    const RENAME_NOREPLACE: u32 = 1 << 0;
    const RENAME_EXCHANGE:  u32 = 1 << 1;
    const RENAME_WHITEOUT:  u32 = 1 << 2;

    trace!(
        "[sys_renameat2] olddirfd: {}, oldpath: {:p}, newdirfd: {}, newpath: {:p}, flags: {:#x}",
        olddirfd, oldpath, newdirfd, newpath, flags
    );

    let proc = current_process();
    let token = proc.get_user_token().await;

    let old_path = translated_str(token, oldpath);
    let new_path = translated_str(token, newpath);

    if old_path.is_empty() || new_path.is_empty() {
        return Err(SysErrNo::ENOENT);
    }

    let old_abs_path = proc.resolve_path_from_fd(olddirfd, &old_path, false).await?;
    let new_abs_path = proc.resolve_path_from_fd(newdirfd, &new_path, false).await?;

    if old_abs_path.len() > PATH_MAX || new_abs_path.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }

    // 查找 old 和 new inode
    let old_inode = find_inode(&old_abs_path, OpenFlags::O_RDWR)?;
    let new_inode_result = find_inode(&new_abs_path, OpenFlags::O_RDWR);

    // 处理 NOREPLACE
    if (flags & RENAME_NOREPLACE) != 0 {
        if new_inode_result.is_ok() {
            warn!("[sys_renameat2] RENAME_NOREPLACE flag set and target exists");
            return Err(SysErrNo::EEXIST);
        }
    }

    // 处理 EXCHANGE
    if (flags & RENAME_EXCHANGE) != 0 {
        old_inode.exchange(&old_abs_path,  &new_abs_path)?;
        return Ok(0);
    }

    // 暂不支持 WHITEOUT
    if (flags & RENAME_WHITEOUT) != 0 {
        warn!("[sys_renameat2] RENAME_WHITEOUT not supported");
        return Err(SysErrNo::EINVAL);
    }

    // 默认重命名行为
    old_inode.rename(&old_abs_path, &new_abs_path)?;

    Ok(0)

}

pub async fn sys_creat(path_ptr: *const u8, mode: u32) -> SyscallRet {
    trace!("[sys_creat] path_ptr: {:p}, mode: {}", path_ptr, mode);

    let proc = current_process();
    let token = proc.memory_set.lock().await.token();
    let path = translated_str(token, path_ptr);
    if path.is_empty() {
        return Err(SysErrNo::ENOENT);
    }
    if path.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }
    let abs_path = if path.starts_with('/') {
        path.clone()
    } else {
        get_abs_path(&proc.cwd.lock().await, &path)
    };
    let open_flags = OpenFlags::O_CREATE | OpenFlags::O_WRONLY | OpenFlags::O_TRUNC;
    let file_class_instance = open_file(&abs_path, open_flags, mode)?;
    let new_fd = proc.fd_table.lock().await.alloc_fd()?;
    proc.fd_table.lock().await.table[new_fd] = Some(file_class_instance);
    Ok(new_fd)
}

pub async fn sys_rmdir(path_ptr: *const u8) -> SyscallRet {
    trace!("[sys_rmdir] path_ptr: {:p}", path_ptr);

    let proc = current_process();
    let token = proc.memory_set.lock().await.token();
    let path = translated_str(token, path_ptr);
    if path.is_empty() {
        return Err(SysErrNo::ENOENT);
    }
    if path.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }
    let abs_path = if path.starts_with('/') {
        path.clone()
    } else {
        get_abs_path(&proc.cwd.lock().await, &path)
    };
    let inode = find_inode(&abs_path, OpenFlags::O_DIRECTORY)?;
    if !inode.is_dir() {
        return Err(SysErrNo::ENOTDIR);
    }
    let (entries, _) = inode.read_dentry(0, 1)?;
    if !entries.is_empty() {
        return Err(SysErrNo::ENOTEMPTY);
    }
    if proc.fd_table.lock().await.find_fd(&abs_path).is_some() {
        return Err(SysErrNo::EBUSY);
    }
    inode.unlink(&abs_path)?;
    remove_inode_idx(&abs_path);
    Ok(0)
}












/// readlinkat 系统调用实现
/// dirfd: 目录文件描述符，或 AT_FDCWD。
/// path_user_ptr: 指向用户空间中符号链接路径的指针。
/// buf_user_ptr: 指向用户空间缓冲区的指针，用于存储链接目标。
/// bufsiz: 用户缓冲区的大小。
/// 返回：成功时为复制到 buf 的字节数，失败时为 -错误码。
pub async fn sys_readlinkat(
    dirfd: i32,
    path_user_ptr: *const u8,
    buf_user_ptr: *mut u8, // buf 是 *mut u8 因为我们要写入它
    bufsize: usize,
) -> SyscallRet {
    log::trace!("[sys_readlinkat] dirfd: {}, path_ptr: {:p}, buf_ptr: {:p}, bufsiz: {}",
                dirfd, path_user_ptr, buf_user_ptr, bufsize);

    let pcb_arc = current_process();
    let token = pcb_arc.get_user_token().await;

    // 1. 校验参数
    if path_user_ptr.is_null() || (buf_user_ptr.is_null() && bufsize > 0) {
        return Err(SysErrNo::EFAULT);
    }
    if bufsize == 0 && !buf_user_ptr.is_null() { // 如果 buf 非空但大小为0，POSIX行为未定，这里返回成功0字节
        return Ok(0);
    }


    // 2. 从用户空间读取路径字符串
    let path_kernel_str = translated_str(token, path_user_ptr);
 
    // 3. 特殊处理 /proc/self/exe 
    if path_kernel_str == "/proc/self/exe" {
        log::debug!("[sys_readlinkat] Handling /proc/self/exe special case.");
        // 假设 ProcessControlBlock 或其 fs_info 有 exe_path() 方法
        let exe_path_kernel_str = pcb_arc.exe.lock().await; // 假设返回 Result<String, SysErrNo>

        let exe_path_bytes = exe_path_kernel_str.as_bytes();
        let len_to_copy = core::cmp::min(exe_path_bytes.len(), bufsize);

        if len_to_copy > 0 { // 只有当有东西可复制且用户缓冲区非空时才复制
            match unsafe {
                copy_to_user_bytes(
                    token,
                    VirtAddr::from(buf_user_ptr as usize),
                    &exe_path_bytes[0..len_to_copy],
                )
            } {
                Ok(copied) => {
                    if copied != len_to_copy { // copy_to_user_bytes 返回实际复制的
                        // 这可能表示用户缓冲区比 len_to_copy 小，但我们已经用了 min
                        // 或者 copy_to_user_bytes 内部有其他限制/错误
                        log::warn!("/proc/self/exe: copy_to_user copied {} instead of {}", copied, len_to_copy);
                        return Err(SysErrNo::EFAULT); // 或者返回部分成功？
                    }
                }
                Err(_) => return Err(SysErrNo::EFAULT),
            }
        }
        return Ok(len_to_copy );
    }

    // 4. 解析路径以获取符号链接本身的 inode (不 follow 最后一个组件)
    //    我们需要 resolve_path_from_fd 的 follow_last_symlink 参数为 false。
    let symlink_abs_path = match pcb_arc.resolve_path_from_fd(
        dirfd,
        &path_kernel_str,
        false, // <--- 重要：不解析路径的最后一个符号链接组件
    ).await {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    // 5. 获取符号链接的 inode
    //    resolve_path_from_fd 如果 follow_last_symlink=false，它返回的路径
    //    应该就是符号链接本身的路径（如果最后一个组件是符号链接）。
    //    然后我们需要 lookup 这个路径来得到 inode。
    //    或者，resolve_path_from_fd 可以直接返回最终的 Arc<OSInode> 和真实路径。
    //    假设我们现在有符号链接的真实路径 symlink_abs_path。
    let symlink_inode_arc = open_file (&symlink_abs_path, OpenFlags::empty(), NONE_MODE)?.file()?;
    let mut linkbuf = vec![0u8; bufsize];
    let readcnt = symlink_inode_arc.inner.lock().inode.read_link(&mut linkbuf, bufsize)?;
    let mut buffer = UserBuffer::new(translated_byte_buffer(token, buf_user_ptr, readcnt));
    buffer.write(&linkbuf);
    Ok(readcnt)
    

}




/// symlinkat 系统调用实现
/// target_user_ptr: 指向用户空间中符号链接目标的字符串指针。
/// newdirfd: 目录文件描述符，用于解析 linkpath。可以是 AT_FDCWD。
/// linkpath_user_ptr: 指向用户空间中要创建的符号链接路径的指针。
/// 返回：成功时为 0，失败时为 -错误码。
pub async fn sys_symlinkat(
    target_user_ptr: *const u8,
    newdirfd: i32, // POSIX 是 int, Rust 通常用 i32
    linkpath_user_ptr: *const u8,
) -> SyscallRet {
    log::trace!("[sys_symlinkat] target_ptr: {:p}, newdirfd: {}, linkpath_ptr: {:p}",
                target_user_ptr, newdirfd, linkpath_user_ptr);

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();

    // 1. 校验用户指针
    if target_user_ptr.is_null() || linkpath_user_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 2. 从用户空间读取 target 和 linkpath 字符串
    let target_kernel_str =   translated_str(token, target_user_ptr );

    let linkpath_kernel_str =  translated_str(token, linkpath_user_ptr );
     

   

    // 3. 解析 linkpath 以确定其父目录和新符号链接的名称
   

    let resolved_linkpath_abs_str = pcb_arc. resolve_path_from_fd(
        
        newdirfd,
        &linkpath_kernel_str,
        false).await?;
        
    


    // 4. 分离出父目录路径和要创建的符号链接的名称
    let (parent_dir_abs_path, symlink_name) =
        get_parent_path_and_filename(&resolved_linkpath_abs_str);

    if symlink_name.is_empty() || symlink_name == "/" || symlink_name == "." || symlink_name == ".." {
        log::warn!("[sys_symlinkat] Invalid symlink name derived: '{}'", symlink_name);
        return Err(SysErrNo::EISDIR); // 或者 EEXIST，或 EINVAL
    }

    log::debug!("[sys_symlinkat] Parent dir: '{}', symlink name: '{}'", parent_dir_abs_path, symlink_name);

    // 5. 检查目标符号链接路径 `resolved_linkpath_abs_str` 是否已存在
    //    使用 lookup_inode，不 follow 符号链接，因为我们想看这个路径本身是否存在。
    if let Ok(_) = open_file(&resolved_linkpath_abs_str, OpenFlags::empty(), NONE_MODE) {
        return Err(SysErrNo::EEXIST);
    }
    // 6. 获取父目录的 inode
    let parent_dir_inode_arc = match find_inode(&parent_dir_abs_path,OpenFlags::O_PATH|OpenFlags::O_DIRECTORY){
        Ok(inode) => inode,
        Err(e) => {
            log::warn!("[sys_symlinkat] Failed to lookup parent directory '{}': {:?}", parent_dir_abs_path, e);
            return Err(e); // 父目录不存在或不可访问
        }
    };

    // // 7. 检查在父目录中创建文件的权限 (通常是父目录的写权限和执行权限)
    // //    你需要一个权限检查函数，例如 parent_dir_inode_arc.check_permission(Permissions::WRITE_EXEC)
    
    // let (uid, gid) = (pcb_arc.get_uid(), pcb_arc.get_gid()); // 假设 PCB 有这些方法
    // // 需要父目录有写和执行权限才能在其中创建文件/链接
    // let mut write_exec_mode = FaccessatMode::empty();
    // write_exec_mode.insert(FaccessatMode::W_OK);
    // write_exec_mode.insert(FaccessatMode::X_OK);
    // if !parent_dir_inode_arc.access(uid, gid, write_exec_mode).await { // 假设 OSInode::access 是 async
    //     log::warn!("[sys_symlinkat] No write/execute permission in parent directory '{}'", parent_dir_abs_path);
    //     return Err(SysErrNo::EACCES);
    // }


    // 8. 调用父目录 inode 的方法来创建符号链接
    
    parent_dir_inode_arc.sym_link( &target_kernel_str,&resolved_linkpath_abs_str)?;
    Ok(0)
   
}




// pub async fn sys_getrandom(buf_ptr: *const u8, buflen: usize, flags: u32) -> SyscallRet {
//     trace!("[sys_getrandom] buf_ptr: {:p}, buflen: {}, flags: {}", buf_ptr, buflen, flags);
//     let proc= current_process();
//     let token = proc.get_user_token().await;

//     if (flags as i32) < 0 {
//         return Err(SysErrNo::EINVAL);
//     }

    

//     if buf_ptr.is_null() {
//         return Err(SysErrNo::EINVAL);
//     }

//     open_device_file("/dev/random")?.read(UserBuffer::new(
//         translated_byte_buffer(token, buf_ptr, buflen),
//     )).await
// }





/// pipe2 系统调用
/// pipefd_user_ptr: 指向用户空间 int[2] 数组的指针，用于接收两个新的文件描述符。
/// flags: 创建管道的标志 (例如 O_CLOEXEC, O_NONBLOCK)。
/// 返回：成功时为 0，失败时为 -错误码。
pub async fn sys_pipe2(pipefd_user_ptr: *mut i32, flags_u32: u32) -> SyscallRet {
    log::trace!("[sys_pipe2]:(pipefd_ptr: {:p}, flags: 0x{:x})", pipefd_user_ptr, flags_u32);

    let pcb_arc = current_process();
    let token = pcb_arc.memory_set.lock().await.token();

    // 1. 校验用户指针
    if pipefd_user_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 2. 解析 flags
    let open_flags = OpenFlags::from_bits_truncate(flags_u32); // 忽略未知标志位
    // TODO: 检查是否有不支持的 flags (pipe2 只支持 O_CLOEXEC, O_DIRECT, O_NONBLOCK)
    //       O_DIRECT 对管道通常无意义。我们主要关心 O_CLOEXEC 和 O_NONBLOCK。

    // 3. 创建管道
    let (read_pipe_arc, write_pipe_arc) = make_pipe(open_flags);

    // 4. 为读端和写端分配文件描述符
    //    add_fd_to_current_process 需要是 async 或在异步上下文中安全调用
   let fd_read= pcb_arc.alloc_and_add_fd(FileDescriptor::new(open_flags, FileClass::Abs(read_pipe_arc))).await?;
    
   let fd_write=  pcb_arc.alloc_and_add_fd(FileDescriptor::new(open_flags, FileClass::Abs(write_pipe_arc))).await?;


    // 5. 将两个文件描述符写回用户空间
    let fds_to_write: [i32; 2] = [fd_read as i32, fd_write as i32];
    match unsafe {
        copy_to_user_bytes_exact(
            token,
            VirtAddr::from(pipefd_user_ptr as usize),
            core::slice::from_raw_parts(
                fds_to_write.as_ptr() as *const u8,
                core::mem::size_of::<[i32; 2]>(),
            ),
        )
    } {
        Ok(_) => {
            log::info!("[sys_pipe2] Successfully created pipe with fds: read={}, write={}", fd_read, fd_write);
            Ok(0)
        }
        Err(_translate_err) => {
            // 写回失败，这是一个严重的问题。
            // 理论上应该尝试关闭已分配的 fd_read 和 fd_write。
            log::error!("sys_pipe2: Failed to copy fds to user space: {:?}", _translate_err);
            // pcb_arc.close_fd(fd_read as usize).await.ok();
            // pcb_arc.close_fd(fd_write as usize).await.ok();
            Err(SysErrNo::EFAULT)
        }
    }
}







/// sendfile 系统调用实现
/// out_fd: 输出文件描述符 (通常是套接字)
/// in_fd: 输入文件描述符 (通常是常规文件)
/// offset_user_ptr: 指向用户空间 off_t (isize) 的指针，或为 0 (NULL)
/// count: 要传输的字节数
/// 返回：成功时为传输的字节数，失败时为 -错误码
pub async fn sys_sendfile(
    out_fd: i32,
    in_fd: i32,
    offset_user_ptr: *mut isize,
    count: usize,
) -> SyscallRet {
    log::trace!("[sys_sendfile] out_fd: {}, in_fd: {}, offset_ptr: {:p}, count: {}",
                out_fd, in_fd, offset_user_ptr, count);

    if count == 0 {
        return Ok(0);
    }

    let proc_arc = current_process(); // 改为 current_process().await
    let token = proc_arc.memory_set.lock().await.token();
    // fd_table 的锁现在在 ProcessControlBlock 的 get_file 方法内部处理 (根据你的原始代码风格)
    // let fd_table_guard = proc_arc.fd_table.lock().await;

    // 1. 获取并校验文件描述符 

    // 校验 fd 范围 (MAX_FD_NUM 需要定义)
    if out_fd < 0 || out_fd as usize >= MAX_FD_NUM ||
       in_fd < 0 || in_fd as usize >= MAX_FD_NUM {
        return Err(SysErrNo::EBADF);
    }

    let out_file_desc_wrapper = proc_arc.get_file(out_fd as usize).await?; // 假设返回 Result<FileDescriptor, SysErrNo>
    let in_file_desc_wrapper = proc_arc.get_file(in_fd as usize).await?;

    // 2. 检查文件权限和类型
    if !offset_user_ptr.is_null()
  {  proc_arc.manual_alloc_type_for_lazy(offset_user_ptr).await?;
  }

    if !out_file_desc_wrapper.writable()? { 
        log::warn!("[sys_sendfile] out_fd {} is not writable", out_fd);
        return Err(SysErrNo::EBADF);
    }
    if !in_file_desc_wrapper.readable()? { 
        log::warn!("[sys_sendfile] in_fd {} is not readable", in_fd);
        return Err(SysErrNo::EBADF);
    }

    // if out_file_desc_wrapper.is_abs() || in_file_desc_wrapper.is_abs() {
    //     return Err(SysErrNo::EINVAL); // 通常 sendfile 不用于抽象/特殊文件
    // }


    let infile = in_file_desc_wrapper;  // 直接使用
    let outfile = out_file_desc_wrapper; // 直接使用


    // 3. 处理 offset_ptr (与之前版本类似，但使用异步 lseek)
    let mut current_read_offset_from_in_file: Option<usize> = None; // Some(offset) 表示从指定偏移读取
    let mut original_in_fd_offset_to_restore: Option<usize> = None; // 如果 offset_user_ptr 为 NULL

    if offset_user_ptr.is_null() {
        
    } else {
        // offset 非 NULL: 从用户提供的 *offset 开始读取，并更新 *offset，in_fd 偏移不变
        let initial_offset_val = match unsafe {
            copy_from_user_exact::<isize>(token, offset_user_ptr) // 从用户空间读 isize
        } {
            Ok(val) => val,
            Err(_) => return Err(SysErrNo::EFAULT),
        };
        if initial_offset_val < 0 {
            return Err(SysErrNo::EINVAL);
        }
        current_read_offset_from_in_file = Some(initial_offset_val as usize);
        // 保存原始偏移，以便之后恢复，因为当 offset_user_ptr 非空时，in_fd 的文件指针不应改变
        original_in_fd_offset_to_restore = Some(infile.lseek(0, SEEK_CUR)? as usize);
    }

    // 4. 主循环
    let mut total_bytes_transferred: usize = 0;
    let mut kernel_transfer_buffer = vec![0u8; SENDFILE_KERNEL_BUFFER_SIZE.min(count)]; // 限制单次读写大小

    while total_bytes_transferred < count {
        let bytes_to_process_this_loop = core::cmp::min(
            count - total_bytes_transferred,
            kernel_transfer_buffer.len(), // 使用已分配缓冲区的大小
        );
        if bytes_to_process_this_loop == 0 {
            break;
        }

        let current_kernel_slice_for_read = &mut kernel_transfer_buffer[0..bytes_to_process_this_loop];

        // a. 从 in_fd 读取数据
        let bytes_read_from_in_fd: usize;
        let mut read_user_buf_vec = Vec::new(); // UserBuffer 需要 Vec<&mut [u8]>
        unsafe {
            read_user_buf_vec.push(core::slice::from_raw_parts_mut(
                current_kernel_slice_for_read.as_mut_ptr(),
                current_kernel_slice_for_read.len(),
            ));
        }
        let in_user_buffer = UserBuffer::new(read_user_buf_vec);

        if let Some(offset_val) = current_read_offset_from_in_file {
            // 从指定偏移读取 (不改变 infile 的持久文件指针)
            infile.lseek(offset_val as isize, SEEK_SET)?;
            bytes_read_from_in_fd = match infile.read(in_user_buffer).await {
                Ok(n) => n,
                Err(e) => { // 读取出错
                    if total_bytes_transferred > 0 { break; } // 如果已传输一些，则返回成功的部分
                    else {
                        // 恢复原始偏移（如果适用）
                        if let Some(orig_off) = original_in_fd_offset_to_restore {
                            infile.lseek(orig_off as isize, SEEK_SET).ok();
                        }
                        return Err(e);
                    }
                }
            };
            // current_read_offset_from_in_file 需要在循环外更新，或者在这里更新并用于下次 lseek
        } else {
            // 从当前文件偏移读取 (会更新 infile 的持久文件指针)
            bytes_read_from_in_fd = match infile.read(in_user_buffer).await {
                Ok(n) => n,
                Err(e) => {
                    if total_bytes_transferred > 0 { break; } else { return Err(e); }
                }
            };
        }

        if bytes_read_from_in_fd == 0 { // EOF on in_fd
            break;
        }

        // b. 将读取到的数据写入 out_fd
        // 构造 UserBuffer for write (这是个痛点，因为 UserBuffer::new 需要 &mut)
        // **这是一个高风险的 `unsafe` 操作，假设 `outfile.write` 不会修改缓冲区**
        // **理想情况下，UserBuffer 或 File::write 应该能接受 `&[u8]`**
        let mut write_user_buf_vec = Vec::new();
        unsafe {
            let kernel_slice_for_write = &kernel_transfer_buffer[0..bytes_read_from_in_fd];
            // 将 &[u8] 强制转换为 &mut [u8] 以匹配 UserBuffer::new 的签名
            // 这是非常不安全的，依赖于 outfile.write 的内部实现。
            let mutable_alias_for_write = core::slice::from_raw_parts_mut(
                kernel_slice_for_write.as_ptr() as *mut u8, // <--- unsafe cast
                kernel_slice_for_write.len()
            );
            write_user_buf_vec.push(mutable_alias_for_write);
        }
        let out_user_buffer = UserBuffer::new(write_user_buf_vec);

        match outfile.write(out_user_buffer).await {
            Ok(bytes_written_to_out_fd) => {
                if bytes_written_to_out_fd != bytes_read_from_in_fd {
                    // 部分写入
                    total_bytes_transferred += bytes_written_to_out_fd;
                    // 更新 current_read_offset_from_in_file (如果正在使用)
                    if let Some(ref mut offset_val) = current_read_offset_from_in_file {
                        *offset_val += bytes_written_to_out_fd;
                    } else {
                        // 如果是更新 infile 的持久偏移，但只写了一部分
                        // infile 的偏移已经前进了 bytes_read_from_in_fd
                        // 我们需要把它回退 (bytes_read_from_in_fd - bytes_written_to_out_fd)
                        let rewind_amount = (bytes_read_from_in_fd - bytes_written_to_out_fd) as isize;
                        if rewind_amount > 0 {
                            infile.lseek(-rewind_amount, SEEK_CUR).ok();
                        }
                    }
                    break; // 结束传输
                }
                total_bytes_transferred += bytes_written_to_out_fd;
            }
            Err(e) => { // 写入 out_fd 失败
                if total_bytes_transferred > 0 { break; }
                else {
                    if let Some(orig_off) = original_in_fd_offset_to_restore { // 恢复offset_ptr!=NULL时的原始偏移
                        infile.lseek(orig_off as isize, SEEK_SET).ok();
                    } else if offset_user_ptr.is_null() && bytes_read_from_in_fd > 0 { // 恢复offset_ptr==NULL时多读的部分
                        infile.lseek(-(bytes_read_from_in_fd as isize), SEEK_CUR).ok();
                    }
                    return Err(e);
                }
            }
        }

        // c. 更新 current_read_offset_from_in_file (如果正在使用)
        if let Some(ref mut offset_val) = current_read_offset_from_in_file {
            *offset_val += bytes_read_from_in_fd; // 更新下一次读取的起始点
        }
    } // end while loop

    // 5. 恢复 in_fd 的原始偏移量 (如果 offset_user_ptr 非空)
    if let Some(orig_off) = original_in_fd_offset_to_restore {
        infile.lseek(orig_off as isize, SEEK_SET).ok();
    }

    // 6. 如果 offset_user_ptr 非空，将最终的 offset 写回用户空间
    if let Some(final_offset_val) = current_read_offset_from_in_file {
        if !offset_user_ptr.is_null() {



            *translated_refmut(token, offset_user_ptr as *mut u64)?=final_offset_val as u64;
            
        }
    }

    Ok(total_bytes_transferred )
}



pub async  fn sys_statfs(_path: *const u8, statfs: *mut Statfs) -> SyscallRet {
    trace!("[sys_statfs] path:{:#?} statfs:{:#?}",_path,statfs);
    current_process().memory_set.lock().await.safe_put_data( statfs, crate::fs::fs_stat()?).await?;
    Ok(0)
}




/// man 2: int truncate(const char *path, off_t length);
pub async fn sys_truncate(path_ptr: *const u8, length: u64) -> SyscallRet {
    let proc = current_process();
    let mut ms = proc.memory_set.lock().await;
    // 从用户指针翻译出路径字符串
    let path = ms.safe_translated_str(path_ptr).await;

    trace!("[sys_truncate] path: {:?}, length: {}", path, length);

    // 1. 验证 length 参数
  

    // 2. 解析为完整路径
    let cwd = proc.cwd.lock().await.clone();
    let full_path = normalize_and_join_path(cwd.as_str(), path.as_str())?;

    // 3. 打开文件以进行写操作，然后立即截断并关闭。
    //    我们需要一个能确保有写权限的打开模式。
    //    O_WRONLY 即可。如果文件不存在，truncate 应该失败，所以不加 O_CREAT。
    let inode = match open_file(&full_path, OpenFlags::O_WRONLY, 0) {
        Ok(inode) => inode,
        // open_file 可能会因为文件不存在(ENOENT)或无权限(EACCES)而失败，
        // 这正是 truncate 应该有的行为。
        Err(e) => return Err(e),
    };

    // 4. 调用 Inode 的 truncate 方法
    if let Err(e) = inode.file()?.truncate(length){
        warn!("[sys_truncate] Failed to truncate file '{}': {:?}", full_path, e);
        return Err(e);
    }

  
    Ok(0)
}


/// man 2: int ftruncate(int fd, off_t length);
pub async fn sys_ftruncate(fd: i32, length: u64) -> SyscallRet {
    trace!("[sys_ftruncate] fd: {}, length: {}", fd, length);

    // 1. 验证 length 参数 (与 truncate 相同)
    // 假设 usize 类型保证了非负。

    // 2. 从文件描述符表中获取文件对象
    let proc = current_process();
    let file = match proc.fd_table.lock().await.get_file(fd as usize) {
        Ok(f) => f,
        Err(e) => return Err(e), // e.g., EBADF for invalid fd
    };
    let file = file.file()?;
    // 3. 检查文件是否以可写模式打开
    //    这是 ftruncate 的一个重要要求。
    if !file.writable().unwrap() {
        // 返回 EINVAL 或 EBADF 都是 man page 中提到的可能错误。
        // EINVAL 更贴切，因为它指的是“不适合此操作”。
        return Err(SysErrNo::EINVAL);
    }
    
    if let Err(e) = file.truncate(length) {
        return Err(e);
    }
    
    Ok(0)
}


/// man 2: int mlock(const void *addr, size_t len);
pub async fn sys_mlock(addr: usize, len: usize) -> SyscallRet {
    trace!("[sys_mlock] addr: {:#x}, len: {}", addr, len);

    // 1. 基本参数验证
    if len == 0 {
        return Err(SysErrNo::EINVAL);
    }
    // man page 说 addr 应该对齐，但内核会处理，所以我们无需检查 addr 的对齐。

    // 2. 获取进程的内存集合 (MemorySet)
    let proc = current_process();
    let memory_set = proc.memory_set.lock().await;

    // 3. 计算需要操作的虚拟页范围
    let start_va = VirtAddr::from(addr);
    let end_va = VirtAddr::from(addr + len - 1);
    
    // 4. 验证内存区域是否已映射
    //    我们需要遍历指定范围内的所有页，确保它们都存在于某个 MapArea 中。
    //    这是 mlock 的核心验证步骤。
    if !memory_set.is_region_alloc(start_va, end_va) {
    
        return Err(SysErrNo::ENOMEM);
    }


  
    
    trace!("[sys_mlock] Region {:#x} - {:#x} locked (no-op).", addr, addr + len);

    // 6. 成功
    Ok(0)
}

/// man 2: int munlock(const void *addr, size_t len);
pub async fn sys_munlock(addr: usize, len: usize) -> SyscallRet {
    trace!("[sys_munlock] addr: {:#x}, len: {}", addr, len);

    // 1. 基本参数验证
    if len == 0 {
        return Err(SysErrNo::EINVAL);
    }

    // 2. 获取进程的内存集合 (MemorySet)
    let proc = current_process();
    let mut memory_set = proc.memory_set.lock().await;

    // 3. 计算需要操作的虚拟页范围
    let start_va = VirtAddr::from(addr);
    let end_va = VirtAddr::from(addr + len - 1);
    
    // 4. 验证内存区域是否已映射 (与 mlock 相同)
    if !memory_set.is_region_alloc(start_va, end_va) {
        return Err(SysErrNo::ENOMEM);
    }

    // 5. (空操作) 解除锁定
    //    在支持换页的系统中，这里会清除 PTE 的 LOCKED 标志。
    //    在我们的实现中，我们什么都不做。
   
    
    trace!("[sys_munlock] Region {:#x} - {:#x} unlocked (no-op).", addr, addr + len);
    
    // 6. 成功
    Ok(0)
}


/// man 2: int mlockall(int flags);
pub async fn sys_mlockall(flags: u32) -> SyscallRet {
    trace!("[sys_mlockall] flags: {}", flags);

    // // 1. 将 u32 转换为我们定义的 bitflags 类型，并验证合法性
    // let mlock_flags = match MlockallFlags::from_bits(flags) {
    //     Some(f) => f,
    //     None => return Err(SysErrNo::EINVAL), // 非法的 flags 组合
    // };
    // // 确保至少设置了一个有效标志
    // if mlock_flags.is_empty() {
    //     return Err(SysErrNo::EINVAL);
    // }

    // // 2. 获取进程的 MemorySet
    // let proc = current_process();
    // let mut memory_set = proc.memory_set.lock().await;

    // // 3. 处理 MCL_CURRENT
    // if mlock_flags.contains(MlockallFlags::MCL_CURRENT) {
    //     // (空操作) 遍历所有已存在的 MapArea 并“锁定”它们
    //     // for area in memory_set.areas.iter_mut() {
    //     //     area.locked = true;
    //     // }
    //     trace!("[sys_mlockall] All current pages locked (no-op).");
    // }

    // // 4. 处理 MCL_FUTURE
    // if mlock_flags.contains(MlockallFlags::MCL_FUTURE) {
    //     // 设置 MemorySet 中的状态标志，以便未来映射时自动锁定
    //     //todo()
    //     // memory_set.mlockall_status |= MlockallFlags::MCL_FUTURE;
    //     trace!("[sys_mlockall] Future pages will be locked.");
    // }

    // 5. 成功
    Ok(0)
}


/// man 2: int munlockall(void);
pub async fn sys_munlockall() -> SyscallRet {
    trace!("[sys_munlockall] Unlocking all pages.");
    
    // 1. 获取进程的 MemorySet
    // let proc = current_process();
    // let mut memory_set = proc.memory_set.lock().await;
    
    // 2. 清除 MCL_FUTURE 状态标志
    // memory_set.mlockall_status.remove(MlockallFlags::MCL_FUTURE);
    
    
    trace!("[sys_munlockall] All pages unlocked (no-op).");
    
    // 4. 成功
    Ok(0)
}
pub fn sys_sync() -> SyscallRet {
    trace!("[sys_sync] Syncing all filesystems (no-op).");
    
    VfsManager::sync();
    Ok(0)
}   
pub async  fn sys_fsync(fd: usize) -> SyscallRet {
    trace!("[sys_fsync] fd: {}", fd);
    let task = current_process();
    let  file = task.get_file(fd).await;
    if  file.is_err() {
        return Err(SysErrNo::EINVAL);
    }

    let file = file.unwrap().file()?;
    file.inner.lock().inode.sync();
    Ok(0)
}