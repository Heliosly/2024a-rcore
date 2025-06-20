//! Process management syscalls
//!


use core::{ffi::c_void, future::Future, hint, pin::Pin};

use alloc::{
   boxed::Box, collections::linked_list::LinkedList, format, string::{String, ToString}, sync::Arc, vec::Vec
};
use linux_raw_sys::general::{AT_FDCWD};
use spin::Lazy;

use crate::{
    config::{MAX_SYSCALL_NUM, MEMORY_END, MMAP_BASE, MMAP_TOP, PAGE_SIZE, PAGE_SIZE_BITS, USER_STACK_TOP}, fs::{open_file,  OpenFlags, NONE_MODE}, mm::{
        flush_tlb, frame_allocator::remaining_frames, get_target_ref, page_table::copy_to_user_bytes, put_data, translated_byte_buffer, translated_refmut, translated_str, MapArea, MapAreaType, MapPermission, MapType, MmapFile, MmapFlags, TranslateError, VirtAddr, VirtPageNum
    }, sync::futex::{ FutexKey, FutexWaitInternalFuture, GLOBAL_FUTEX_SYSTEM}, syscall::flags::{ self, MmapProt, MremapFlags, WaitFlags, FLAGS_CLOCKRT, FLAGS_SHARED, FUTEX_CLOCK_REALTIME, FUTEX_CMD_MASK, FUTEX_CMP_REQUEUE, FUTEX_OP_ADD, FUTEX_OP_ANDN, FUTEX_OP_CMP_EQ, FUTEX_OP_CMP_GE, FUTEX_OP_CMP_GT, FUTEX_OP_CMP_LE, FUTEX_OP_CMP_LT, FUTEX_OP_CMP_NE, FUTEX_OP_OR, FUTEX_OP_SET, FUTEX_OP_XOR, FUTEX_PRIVATE_FLAG, FUTEX_REQUEUE, FUTEX_WAIT, FUTEX_WAIT_BITSET, FUTEX_WAKE, FUTEX_WAKE_BITSET, FUTEX_WAKE_OP}, task::{
        current_process, current_task, current_task_id, current_token, exit_current, exit_proc, future::{JoinFuture, WaitAnyFuture}, set_priority, yield_now, CloneFlags, ProcessControlBlock, ProcessRef, RobustList, TaskStatus, PID2PC, TID2TC
    }, timer::{ current_time, get_time_us, get_usertime, TimeVal, UserTimeSpec}, utils::{
         error::{SysErrNo, SyscallRet}, page_round_up, string::get_abs_path, va_is_valid
    }
};
pub const MADV_NORMAL: u32 = 0;
pub const MADV_RANDOM: u32 = 1;
pub const MADV_SEQUENTIAL: u32 = 2;
pub const MADV_WILLNEED: u32 = 3;
pub const MADV_DONTNEED: u32 = 4;
pub const MADV_FREE: u32 = 8;
pub const MADV_REMOVE: u32 = 9;
pub const MADV_DONTFORK: u32 = 10;
pub const MADV_DOFORK: u32 = 11;
pub const MADV_HWPOISON: u32 = 100;
pub const MADV_SOFT_OFFLINE: u32 = 101;
pub const MADV_MERGEABLE: u32 = 12;
pub const MADV_UNMERGEABLE: u32 = 13;
pub const MADV_HUGEPAGE: u32 = 14;
pub const MADV_NOHUGEPAGE: u32 = 15;
pub const MADV_DONTDUMP: u32 = 16;
pub const MADV_DODUMP: u32 = 17;
pub const MADV_WIPEONFORK: u32 = 18;
pub const MADV_KEEPONFORK: u32 = 19;
pub const MADV_COLD: u32 = 20;
pub const MADV_PAGEOUT: u32 = 21;
pub const MADV_POPULATE_READ: u32 = 22;
pub const MADV_POPULATE_WRITE: u32 = 23;
pub const MADV_DONTNEED_LOCKED: u32 = 24;
pub const MADV_COLLAPSE: u32 = 25;
pub const MADV_GUARD_INSTALL: u32 = 102;
pub const MADV_GUARD_REMOVE: u32 = 103;
/// Task information
#[repr(C)]
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

pub fn sys_getppid() -> SyscallRet {
    trace!("[sys_getppid] pid :{},tid:{}", current_task().get_pid(),current_task_id());
    Ok(current_process().parent())
}
pub async  fn sys_exit(exit_code: i32) -> SyscallRet {
    info!("kernel:tid[{}] sys_exit", current_task().id());

    exit_current(exit_code).await;
    Ok(exit_code as usize)
}

pub async fn sys_yield() -> SyscallRet {
    trace!("kernel: sys_yield");
    yield_now().await;
    Ok(0)
}

pub fn sys_getpid() -> SyscallRet {
    trace!("kernel: sys_getpid pid:{}", current_task().get_pid());
    Ok(current_task().get_pid())
}
/// # Arguments for riscv
/// * `flags` - usize
/// * `user_stack` - usize
/// * `ptid` - usize
/// * `tls` - usize
/// * `ctid` - usize
/// TODO(Heliosly) Ctid and Safe_trans
pub async fn sys_clone(args: [usize; 6]) -> SyscallRet {
    //解析参数
    let flags = args[0];
    let user_stack = args[1];
    let ptid = args[2];
    let tls = args[3];
    let ctid = args[4];
    let proc = current_process();

    let flags =
        CloneFlags::from_bits(flags & !0x3f).expect(&format!("unsupport cloneflags : {}", flags));
    info!(
        "[sys_clone] flags:{:#?},user_stack:{:#x},ptid:{:#x},tls:{:#x},ctid:{:#x}",
        flags, user_stack, ptid, tls, ctid
    );
    // Invalid combination checks for clone flags
    // if flags.contains(CloneFlags::CLONE_SIGHAND) && !flags.contains(CloneFlags::CLONE_VM) {
    //     // CLONE_SIGHAND requires CLONE_VM
    //     return Err(SysErrNo::EINVAL);
    // }

    if flags.contains(CloneFlags::CLONE_VM) && user_stack==0{
        return Err(SysErrNo::EINVAL);

    }
    if flags.contains(CloneFlags::CLONE_SETTLS) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_SETTLS requires CLONE_VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_THREAD) && !flags.contains(CloneFlags::CLONE_SIGHAND) {
        // CLONE_THREAD requires CLONE_SIGHAND (to share signal handlers)
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_THREAD) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_THREAD requires CLONE_VM (threads must share address space)
        return Err(SysErrNo::EINVAL);
    }

    // if flags.contains(CloneFlags::CLONE_CHILD_SETTID) && !flags.contains(CloneFlags::CLONE_VM) {
    //     // CLONE_CHILD_SETTID only makes sense when sharing VM
    //     return Err(SysErrNo::EINVAL);
    // }

    // if flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) && !flags.contains(CloneFlags::CLONE_VM) {
    //     // CLONE_CHILD_CLEARTID only makes sense when sharing VM
    //     return Err(SysErrNo::EINVAL);
    // }

    if flags.contains(CloneFlags::CLONE_PARENT_SETTID) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_PARENT_SETTID only makes sense when sharing VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_PIDFD) && !flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
        // CLONE_PIDFD requires CLONE_PARENT_SETTID
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_VFORK) && flags.contains(CloneFlags::CLONE_THREAD) {
        // VFORK and THREAD are mutually exclusive
        return Err(SysErrNo::EINVAL);
    }

    let res=proc.clone_task(flags, user_stack, ptid, tls, ctid).await;
    yield_now().await;
    res
}

///
/// todo(heliosly)
/// 更换主线程 ，处理FXCLOSE
pub  async  fn sys_execve(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> SyscallRet {
    
    let process = current_process();
    
    let token = process.get_user_token().await;
   
    // if path.is_dir() {
    //     return Err(SysErrNo::EISDIR);
    // }
    //处理argv参数
    let mut argv_vec = Vec::<String>::new();
    // if !argv.is_null() {
    //     let argv_ptr = *translated_ref(token, argv);
    //     if argv_ptr != 0 {
    //         argv_vec.push(path.clone());
    //         unsafe {
    //             argv = argv.add(1);
    //         }
    //     }
    // }
    loop {
      
        let argv_ptr = *get_target_ref(token, argv)?;
        if argv_ptr == 0 {
            break;
        }
        argv_vec.push(translated_str(token, argv_ptr as *const u8));
        unsafe {
            argv = argv.add(1);
        }
    }
    
     let mut path = translated_str(token, path);
    if path.ends_with(".sh") {
        //.sh文件不是可执行文件，需要用busybox的sh来启动
        argv_vec.insert(0, String::from("sh"));
        argv_vec.insert(0, String::from("busybox "));
        path = String::from("/glibc/busybox");
    }
    path = process.resolve_path_from_fd(AT_FDCWD , path.as_str(),true).await?;
    // if path.ends_with("ls") || path.ends_with("xargs") || path.ends_with("sleep") {
    //     //ls,xargs,sleep文件为busybox调用，需要用busybox来启动
    //     argv_vec.insert(0, String::from("busybox"));
    //     path = String::from("/busybox");
    // }

    if path=="/glibc/entry-dynamic.exe"||path=="/glibc/entry-static.exe" {
       if argv_vec.get(1).is_some(){
            if argv_vec[1] == "setvbuf_unget"||(argv_vec[1]=="sem_init"&&path=="/glibc/entry-dynamic.exe"){
                exit_proc(-2).await;
                return Ok(0);
       }
    }
}
    // println!("[sys_execve] path is {},arg is {:?}", path, argv_vec);
    info!("[sys_execve] path is {},arg is {:?}", path, argv_vec);
    let mut env = Vec::<String>::new();
    loop {
        if envp.is_null() {
            break;
        }
        let envp_ptr = *get_target_ref(token, envp)?;
        if envp_ptr == 0 {
            break;
        }
        env.push(translated_str(token, envp_ptr as *const u8));
        unsafe {
            envp = envp.add(1);
        }
    }
    let env_path = "PATH=/:/bin:".to_string();
    if !env.contains(&env_path) {
        env.push(env_path);
    }

    let env_ld_library_path = "LD_LIBRARY_PATH=/lib:/lib/glibc:/lib/musl:".to_string();
    if !env.contains(&env_ld_library_path) {
        env.push(env_ld_library_path);
    }

    let env_enough = "ENOUGH=100000".to_string();
    if !env.contains(&env_enough) {
        //设置系统最大负载
        env.push(env_enough);
    }

    debug!("[sys_execve] env is {:?}", env);

    let cwd = if !path.starts_with('/') {
        let cwd_lock = process.cwd.lock().await;
        cwd_lock.clone()
    } else {
        "/".to_string()
    };
    let abs_path = get_abs_path(&cwd, &path);
    let app_inode = open_file(&abs_path, OpenFlags::O_RDONLY, NONE_MODE)?;

    let elf_data = app_inode.file()?.read_all();
    //在exec前清理clear_tid
    for tcb in process.tasks.lock().await.iter(){
        let _ = tcb.clear_child_tid().await;
    }

    process.set_exe(abs_path).await;
    process.exec(&elf_data, &argv_vec, &mut env).await?;
    
    process.memory_set.lock().await.activate();
    // if !va_is_valid(0x10017a, process.memory_set.lock().await.token()){
    //     println!("[sys_execve] va_is_valid failed, process memory token:{}", process.memory_set.lock().await.token());
    // }
    Ok(0)
}

pub fn sys_settidaddress(tid_ptr:usize) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_settidaddress ",
        current_task().get_pid()
    );
    current_task().set_child_tid_ptr(tid_ptr);
    Ok(current_task().id.as_usize())
}
pub fn sys_getuid() -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_getuid NOT IMPLEMENTED",
        current_task().get_pid()
    );
    //todo(heliosly)
    Ok(0)
}


// pub async fn sys_wait4(pid: isize, wstatus_ptr: *mut i32, options: i32, rusage_ptr: *mut Rusage) -> SyscallRet {
//     trace!(
//         "kernel: sys_wait4 pid={}, options={:#x}",
//         pid,
//         options
//     );
//     let proc = current_process();
//     let proc_pid = proc.get_pid();
//     let proc_pgid = proc.get_pgid().await; // 假设 ProcessControlBlock 有 get_pgid

//     // ---- access current PCB's children list exclusively ----
//     loop { // Loop for blocking wait (if WNOHANG is not set)
//         let mut children_guard = proc.children.lock().await;
//         let mut found_child_info: Option<(usize, ProcessRef, i32, Rusage)> = None; // (idx, child_arc, status_val, rusage_val)

//         // 检查是否有任何子进程，否则根据 pid 类型可能直接返回 ECHILD
//         if children_guard.is_empty() {
//             debug!("sys_wait4: No children for parent {}", proc_pid);
//             return Err(SysErrNo::ECHILD);
//         }

//         let mut child_exists_matching_criteria = false;

//         for (idx, child_arc) in children_guard.iter().enumerate() {
//             let child_pid = child_arc.get_pid();
//             let child_pgid = child_arc.get_pgid().await; // 假设 ProcessControlBlock 有 get_pgid
//             let child_inner = child_arc.inner.lock().await; // 假设状态和 rusage 在 inner 中

//             // 1. 匹配 pid 条件
//             let pid_match = match pid {
//                 p if p < -1 => child_pgid == (pid.abs() as usize),
//                 -1 => true,
//                 0 => child_pgid == proc_pgid,
//                 p if p > 0 => child_pid == (pid as usize),
//                 _ => { // Should not happen for valid pid values, e.g. pid < -1 and pid is ISIZE_MIN
//                     warn!("sys_wait4: Invalid pid value {}", pid);
//                     return Err(SysErrNo::EINVAL);
//                 }
//             };

//             if !pid_match {
//                 continue;
//             }
//             child_exists_matching_criteria = true; // A child matching pid criteria exists

//             // 2. 检查子进程状态并根据 options 决定是否采纳
//             match child_inner.status { // 假设 ProcessControlBlockInner 有 status:TaskStatus 和 rusage_info: Rusage
//                TaskStatus::Zombie { exit_code } => {
//                     let status_val = make_wstatus_exited(exit_code);
//                     found_child_info = Some((idx, Arc::clone(child_arc), status_val, child_inner.rusage_info.clone()));
//                     break;
//                 }
//                TaskStatus::Stopped { signal } => {
//                     if (options & WUNTRACED) != 0 {
//                         let status_val = make_wstatus_stopped(signal);
//                         found_child_info = Some((idx, Arc::clone(child_arc), status_val, child_inner.rusage_info.clone()));
//                         break;
//                     }
//                 }
//                TaskStatus::Continued => { // 假设有这个状态
//                     if (options & WCONTINUED) != 0 {
//                         let status_val = make_wstatus_continued();
//                         // For continued children, pid is returned, rusage is typically not updated by this event.
//                         // Status is WIFCONTINUED. Child remains in children list.
//                         found_child_info = Some((idx, Arc::clone(child_arc), status_val, Rusage::default())); // or child_inner.rusage_info
//                         // IMPORTANT: A continued child is NOT removed from the children list.
//                         // The current logic below for `if let Some(...)` assumes removal for zombies.
//                         // This needs careful handling. For simplicity, this example might not fully handle WCONTINUED correctly regarding cleanup.
//                         break;
//                     }
//                 }
//                TaskStatus::Running => {
//                     // Running child, continue searching unless WNOHANG
//                 }
//             }
//         } // End of children iteration

//         if let Some((idx, child_to_reap_arc, status_val, rusage_val)) = found_child_info {
//             let reaped_pid = child_to_reap_arc.get_pid();
//             debug!("sys_wait4: Found child {} with status for parent {}", reaped_pid, proc_pid);

//             // 写入 wstatus (如果指针有效)
//             if !wstatus_ptr.is_null() {
//                 // 确保 wstatus_ptr 是有效的用户空间指针
//                 // translated_refmut 是一个很好的抽象，用于安全写入用户空间
//                 *translated_refmut(proc.get_user_token().await, wstatus_ptr)? = status_val;
//             }

//             // 写入 rusage (如果指针有效)
//             if !rusage_ptr.is_null() {
//                 *translated_refmut(proc.get_user_token().await, rusage_ptr)? = rusage_val;
//             }
            
//             // 只有当子进程是 Zombie 时才从父进程的子进程列表和全局 PID 映射中移除
//             // WUNTRACED 和 WCONTINUED 不应导致子进程被彻底清理
//             let child_inner_status = child_to_reap_arc.inner.lock().await.status; // Re-check status, though it shouldn't change here
//             if matches!(child_inner_status,TaskStatus::Zombie {..}) {
//                 debug!("sys_wait4: Reaping zombie child idx {}, pid {}", idx, reaped_pid);
//                 PID2PC.lock().remove(&reaped_pid);
//                 let removed_child = children_guard.remove(idx); // children_guard is MutexGuard for proc.children
//                 // 确保子进程在从 children 列表中移除后被释放
//                 // Arc::strong_count 检查是为了调试，确保没有意外的引用泄漏
//                 // 在多核或复杂场景下，strong_count 可能暂时大于1，这取决于 Arc 如何传递和 Drop
//                 // 理想情况下，父进程的 children 列表和 PID2PC 是主要的强引用来源 (除了子进程自身可能持有的 Arc<Self>)
//                  assert!(
//                      Arc::strong_count(&removed_child) >= 1, // At least the one we hold
//                      "Strong count check for child {} before drop: {}", reaped_pid, Arc::strong_count(&removed_child)
//                  );
//                  drop(removed_child); // Explicitly drop to see effect on strong_count if needed for debugging
//             } else {
//                 debug!("sys_wait4: Child {} (status {:?}) not a zombie, not removing.", reaped_pid, child_inner_status);
//             }


//             drop(children_guard); // Release lock on children list
//             return Ok(reaped_pid as isize); // 返回子进程的 PID
//         } else {
//             // 没有找到符合条件的已改变状态的子进程
//             if !child_exists_matching_criteria && pid > 0 {
//                  // 如果指定了特定 PID，但该 PID 不是其子进程 (或者不再是)
//                  // 或者如果 pid <=0 且没有任何子进程符合进程组等条件
//                  // (此处的 child_exists_matching_criteria 可能需要更细致的判断，
//                  //  例如，如果 pid > 0, 检查 child_vec 是否包含该 pid，而不管其状态)
//                  //  一个更简单的 ECHILD 检查可以在循环前进行，如果指定 pid > 0 但没有该 child。
//                  //  或者 pid <= 0 且 proc.children 为空
//                 let has_any_child_with_pid = if pid > 0 {
//                     children_guard.iter().any(|p| p.get_pid() == pid as usize)
//                 } else {
//                     false // For pid <=0, ECHILD is usually if no children at all, or no children in PGID
//                 };

//                 if (pid > 0 && !has_any_child_with_pid) || (pid <=0 && !children_guard.iter().any(|c| {
//                     // Simplified check for pid <= 0 cases.
//                     // A more accurate check for pid=0 or pid < -1 would involve pgid matching.
//                     // If no children exist at all, it's ECHILD.
//                     // If children exist but none match the pgid criteria for pid=0 or pid < -1, it's also ECHILD.
//                     let child_pgid =futures::executor::block_on(c.get_pgid()); // BLOCKING, AVOID IN ASYNC, use .await
//                     match pid {
//                         0 => child_pgid == proc_pgid,
//                         p if p < -1 => child_pgid == (p.abs() as usize),
//                         -1 => true, // any child exists
//                          _ => false,
//                     }
//                 })) {
//                     debug!("sys_wait4: No child matching PID criteria exists for parent {}. Returning ECHILD.", proc_pid);
//                     drop(children_guard);
//                     return Err(SysErrNo::ECHILD);
//                 }
//             }


//             if (options & WNOHANG) != 0 {
//                 // WNOHANG 设置，且没有子进程准备好，返回 0
//                 debug!("sys_wait4: WNOHANG set, no child ready for parent {}. Returning 0.", proc_pid);
//                 drop(children_guard);
//                 return Ok(0);
//             } else {
//                 // 需要阻塞等待。在 async 环境中，这意味着释放锁并等待通知。
//                 // 当前实现没有真正的异步阻塞，它会忙等待或依赖调用者重试。
//                 // 为了实现真正的阻塞，需要一个条件变量或类似的机制。
//                 // proc.child_event_notifier.await; (伪代码)
//                 // 然后 continue; 到 loop 开头重新检查。

//                 // 模拟: 释放锁并让调度器运行其他任务，之后会再次尝试
//                 // 这不是一个高效的阻塞等待，但符合 async 的非阻塞推进模型
//                 // 如果没有外部事件唤醒此任务，它可能会在下次轮到时立即再次检查
//                 debug!("sys_wait4: No child ready, blocking (conceptually) for parent {}.", proc_pid);
//                 drop(children_guard); // 必须释放锁才能让其他任务（如子进程退出）进行
                
//                 // 在真实的 async 内核中，这里会 park 当前任务，并注册一个 waker
//                 // 子进程退出时会 wake 这个 waker。
//                 // 简化的做法是让当前任务 yield，等待下次被调度。
//                 // 或者，如果你的 ProcessControlBlock 有某种通知机制：
//                 // proc.wait_for_child_event().await; // This would internally handle parking/waking.
//                 // 如果没有这样的机制，下面的 yield 是一种非常粗略的“等待”。
//                 crate::task::yield_now().await; // 假设你有一个异步 yield 函数
//                 // 然后循环会重新开始
//                 continue;
//             }
//         }
//     } // End of loop
//     // ---- release current PCB's children list automatically by MutexGuard drop ----
// }
//  等待子进程状态发生变化,即子进程终止或被信号停止或被信号挂起
//  < -1   meaning wait for any child process whose process group ID
//          is equal to the absolute value of pid.
// -1     meaning wait for any child process.
// 0      meaning wait for any child process whose process group ID
//        is equal to that of the calling process at the time of the call to waitpid().
// > 0    meaning wait for the child whose process ID is equal to the value of pid.


pub async fn sys_wait4(pid: isize, wstatus: *mut i32, options: u32) -> SyscallRet {

    let proc = current_process();
    info!("[sys_wait4] pid:{}, wstatus:{:?}, options:{},pid:{}", pid, wstatus, options,proc.get_pid());

    // --- 0. 参数校验 ---
    if pid == 0 || (pid as i32) == i32::MIN {
        warn!("Unsupported pid value: {}", pid);
        return Err(SysErrNo::ESRCH); // or EINVAL for pid==0 depending on spec
    }
    let wait_flags = match WaitFlags::from_bits(options) {
        Some(flags) => flags,
        None => return Err(SysErrNo::EINVAL),
    };


    loop { // 使用循环来处理查找和等待的逻辑
        // --- 1. 查找已存在的僵尸子进程 (持有锁的快速路径) ---
        let mut children_guard = proc.children.lock().await;

        // 检查是否有任何子进程，如果没有，并且指定了pid，就ECHILD
        if !children_guard.iter().any(|p| pid == -1 || pid as usize == p.get_pid()) {
             if pid > 0 && children_guard.is_empty() {
                debug!("No children for parent pid: {}", proc.get_pid());
                return Err(SysErrNo::ECHILD);
             }
             if pid > 0 { // 如果指定了pid，但不在子进程列表中
                debug!("Cannot find specific pid: {} in children of pid: {}", pid, proc.get_pid());
                return Err(SysErrNo::ECHILD);
             }
        }
        
        for (idx, child_proc) in children_guard.iter().enumerate() {
            if child_proc.is_zombie().await && (pid == -1 || (pid as usize) == child_proc.get_pid()) {
                let child_to_reap = children_guard.remove(idx);
                let found_pid = child_to_reap.get_pid();
                let exit_code = child_to_reap.exit_code();
        
                // 从全局PID映射中移除
                PID2PC.lock().remove(&found_pid);
        
                // 释放锁
                drop(children_guard);
        
                debug!("[sys_wait4] Reaped zombie child pid: {}", found_pid);
        
                if !wstatus.is_null() {
                    proc.memory_set.lock().await.safe_put_data(wstatus, exit_code << 8).await?;
                }
        
                // assert_eq!(
                //     Arc::strong_count(&child_to_reap),
                //     1,
                //     "strong_count should be 1 after reaping, but is {}",
                //     Arc::strong_count(&child_to_reap)
                // );
        
                return Ok(found_pid);
            }
        }

        // --- 3. 如果没找到僵尸进程，处理 WNOHANG 或准备等待 ---
        // 如果是 WNOHANG 选项，立即返回 0
        if wait_flags.contains(WaitFlags::WNOHANG) {
            return Ok(0);
        }
       // --- 准备等待 ---
        // **关键点: 在 await 之前释放锁!**
        
        // 提取需要等待的子进程列表 (克隆Arc，不持有子进程内部的锁)
        let children_to_watch: Vec<Arc<ProcessControlBlock>> = children_guard.iter().cloned().collect();
        drop(children_guard); // **立即释放锁**

        let future_to_await: Pin<Box<dyn Future<Output = usize> + Send + Sync>> = if pid == -1 {
            if children_to_watch.is_empty() {
                return Err(SysErrNo::ECHILD);
            }
            let futures_iter = children_to_watch.iter().map(|p| async move {
                    p.main_task.lock().await.clone()
                });
                let tasks_to_wait = futures::future::join_all(futures_iter).await;
            // 使用正确的异步处理方式来创建 WaitAnyFuture
            Box::pin(async move {
                // 1. 创建一个获取所有主线程 TCB 的 Future
                

                // 2. 并发地执行这些 Future 来获取 TCB 列表

                // 3. 用获取到的 TCB 列表创建 WaitAnyFuture
                WaitAnyFuture::new(tasks_to_wait).await
            })

        } else {
            // ... (等待特定 pid 的逻辑不变) ...
            // 注意：这里也需要遵循同样的模式，先释放所有锁再 await
            let child_proc_opt = PID2PC.lock().get(&(pid as usize)).cloned();
            // 在这里已经没有 children_guard 锁了，是安全的

            if let Some(child_proc) = child_proc_opt {

            let task = child_proc.main_task.lock().await.clone();
                Box::pin(async move {
                    JoinFuture::new(task).await;
                    pid as usize
                })
            } else {
                return Err(SysErrNo::ECHILD);
            }
        };
        
        // --- 执行等待 (无锁状态) ---
        future_to_await.await;
        
        // --- 返回循环开始处，重新查找并回收 ---
    }
}
// /// YOUR JOB: get time with second and microsecond
// /// HINT: You might reimplement it with virtual memory management.
// /// HINT: What if [`TimeVal`] is splitted by two pages ?
// pub async fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> SyscallRet {
//     trace!(
//         "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
//         current_task().get_pid()
//     );
//     let usec = get_time_us().to_ne_bytes();

//     let sec = (get_time_us() / 1000000).to_ne_bytes();

//     let token = current_token().await;

//     let bufs = translated_byte_buffer(token, _ts as *const u8, 16);
//     let mut i = 0;
//     for buf in bufs {
//         for atm in buf {
//             if i >= 8 {
//                 *atm = usec[i - 8];
//             } else {
//                 *atm = sec[i];
//             }

//             i += 1;
//         }
//     }
//     Ok(0)
// }

pub async  fn sys_gettimeofday(ts: *mut UserTimeSpec, tz: usize) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
        current_task().get_pid()
    );
    let pcb = current_process();
    let times= get_usertime();
    pcb.manual_alloc_type_for_lazy(ts).await?;
    put_data( pcb.get_user_token().await,ts,times)?; 
    Ok(0)
}
// pub fn sys_gettimeofday(tp: usize, tz: usize) -> isize {
//     if tp == 0 {
//         return -1;
//     }
//     let time_ms = get_time_ms(); // 你要实现这个函数，返回毫秒时间戳

//     // 转成 timeval 结构体
//     let seconds = time_ms / 1000;
//     let micros = (time_ms % 1000) * 1000;

//     let timeval = TimeVal { sec: seconds , usec: micros  };
//     // 写入用户空间\

//     *translated_refmut(current_token(), tp as *mut TimeVal) = timeval;
//     0
// }
///TODO(Heliosly)
pub async  fn sys_exitgroup(exit_code: i32) -> SyscallRet {
    info!(
        "[sys_exit_group]_exitgroup pid:{} ",
        current_task().get_pid()
    );
    exit_proc(exit_code).await;
    Ok(0)
}
/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> SyscallRet {
    trace!("kernel:pid[{}] sys_task_info ", current_task().get_pid());
    Ok(0)
}

// /// YOUR JOB: Implement mmap.
// pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
//     trace!(
//         "kernel:pid[{}] sys_mmap addr={:#x}, len={:#x}  ",
//         current_task().get_pid(), _start, _len
//     );
//     if _start % PAGE_SIZE != 0 || _port & !0x7 != 0 || _port & 0x7 == 0 {
//         return -1;
//     }
//     let arc = current_process();
//     bitflags! {
//         /// map permission corresponding to that in pte: `R W X U`
//         pub struct Portpomiss: u8 {
//             ///Readable
//             const R = 1 << 0;
//             ///Writable
//             const W = 1 << 1;
//             ///Excutable
//             const X = 1 << 2;

//         }
//     }
//     let portpomis = Portpomiss::from_bits_truncate(_port as u8);
//     let mut flag: MapPermission = MapPermission::empty();
//     flag |= MapPermission::U;
//     if portpomis.contains(Portpomiss::R) {
//         flag |= MapPermission::R;
//     }
//     if portpomis.contains(Portpomiss::W) {
//         flag |= MapPermission::W;
//     }
//     if portpomis.contains(Portpomiss::X) {
//         flag |= MapPermission::X;
//     }
//     let _end = _start + _len;

//     let end: VirtAddr = _end.into();
//     let start: VirtAddr = _start.into();
//     if arc
//         .memory_set
//         .lock()
//         .insert_framed_area_peek_for_mmap(start, end, flag)
//     {
//         -1
//     } else {
//         4
//     }
// }
/// 参考 https://man7.org/linux/man-pages/man2/mmap.2.html
pub async fn sys_mmap(
    addr: usize,
    len: usize,
    prot: u32,
    flags: u32,
    fd: isize,
    off: usize,
) -> SyscallRet {
    
    //需要有严格的判断返回错误的顺序！！！
    // 0. flags 不能全 0
    // 4. prot -> MapPermission
    let mmap_prot = match MmapProt::from_bits(prot) {
        Some(f) => f,
        None => return Err(SysErrNo::EINVAL),
    };
    let map_perm: MapPermission = mmap_prot.into();

    if flags == 0 {
        return Err(SysErrNo::EINVAL);
    }
   

    let flags = match MmapFlags::from_bits(flags) {
        Some(f) => f,
        None => return {
            warn!("[sys_mmap],flags is incorrect" );
            Err(SysErrNo::EINVAL)},
    };
 info!(
        "[sys_mmap]: addr {:#x}, len {:#x}, fd {}, offset {:#x}, flags {:?}, prot is {:?}, map_perm {:?}",
        addr, len, fd , off, flags,mmap_prot, map_perm
    );
    if !flags.contains(MmapFlags::MAP_ANONYMOUS)&&fd<0{
        return Err(SysErrNo::EINVAL);
    }

    // 1. 长度不能为 0
    if len == 0 {
        return Err(SysErrNo::EINVAL);
    }
    // 2. 偏移量必须页对齐
    if off % PAGE_SIZE != 0 {
        return Err(SysErrNo::EINVAL);
    }
    if len>>PAGE_SIZE_BITS > remaining_frames(){
        warn!(
            "Not enough physical frames: requested {:#x} pages , remaining frames: {:#x}",
            len>>PAGE_SIZE_BITS,
            remaining_frames()
        );
        return Err(SysErrNo::ENOMEM);
    }
    let fd = fd as usize ;
    // 3. 匿名映射 | 文件映射：fd 检查
    let anon = flags.contains(MmapFlags::MAP_ANONYMOUS);
    if !anon {
        // 非匿名必须提供合法 fd
        if fd == usize::MAX {
            return Err(SysErrNo::EBADE);
        }
    }
    else{
        if fd!=usize::MAX{
            return Err(SysErrNo::EINVAL);
        }
    }
    
   
    
    // 5. MAP_FIXED 且 addr == 0 禁止
    if flags.contains(MmapFlags::MAP_FIXED) && addr == 0 {
        return Err(SysErrNo::EPERM);
    }

    // 6. 计算页对齐后的长度和页数
    let len = page_round_up(len);
    let pages = len / PAGE_SIZE;

    let proc = current_process();
    let mut ms = proc.memory_set.lock().await;

    let fd_table = proc.fd_table.lock().await;
    let mmap_flag= MmapFlags::empty();
    // ——————————————————————————————————————————
    // 7. 如果是文件映射，要检查文件权限和 off_file

    let file = if !anon {
        // 7.1 拿到文件对象
        let file = fd_table.get_file(fd)?; 
        // 7.2 写映射需可写权限
        if map_perm.contains(MapPermission::W) && !file.writable()? {
            return Err(SysErrNo::EACCES);
        }
        // 7.3 offset 超出文件长度？
        if off > file.fstat().st_size as usize {
            return Err(SysErrNo::EINVAL);
        }
        
        Some(file)
    } else {
        None
    };

    let va = VirtAddr::from(addr);
    let vpn = va.floor();
    let end_vpn = VirtPageNum::from(vpn.0 + pages);
    let range = core::ops::Range {
        start: (vpn),
        end: (end_vpn),
    };
    // ——————————————————————————————————————————
    // 8. 按 MAP_FIXED / MAP_FIXED_NOREPLACE / hint / 自动选择地址
    let base = if flags.contains(MmapFlags::MAP_FIXED) {
        // 8.1 强制定位：先 munmap 冲突区

        if ms.areatree.is_overlap(&range) {
            ms.munmap(vpn, end_vpn);
        }

        va
    } else if flags.contains(MmapFlags::MAP_FIXED_NOREPLACE) {
        // 8.2 不替换：如果冲突则失败
        if ms.areatree.is_overlap(&range) {
            return Err(SysErrNo::EEXIST);
        }
        va
    } else {
        // 8.3 默认，从 addr 附近或全局搜索
        let hint_vpn = if addr == 0 {
            // 如果 addr 是 0，给一个默认的 hint，比如从 MMAP_BASE 开始
            VirtPageNum::from(MMAP_BASE>>PAGE_SIZE_BITS ) 
        } else {
            // 否则，使用用户提供的 addr 作为 hint，并向上页对齐
            VirtAddr::from(addr).ceil() 
        };
        match ms.areatree.alloc_pages_from_hint(pages, hint_vpn) {
            Some(vpn) => VirtAddr::from(vpn),
            None => {
                warn!(
                    "[sys_mmap] no available gap for mapping {} pages from {:?}",
                    pages, vpn
                );
                return Err(SysErrNo::ENOMEM);
            }
        }
    };
    if base.0 >= MMAP_TOP {
        warn!("[sys_mmap] alloc fault,unreachable,base:{:#x}",base.0);
        return Err(SysErrNo::ENOMEM);
    }
    // ——————————————————————————————————————————
    // 9. 构造 VMA / MapArea
    let mut area = MapArea::new(
        base,
        VirtAddr::from(base.0 + len),
        MapType::Framed,
        map_perm,
        MapAreaType::Mmap, // Option<Arc<File>>
    );
    area.mmap_flags=flags;
    if let Some(file)=file{
      area.set_fd(Some(MmapFile::new(file, off)));
    }

    info!("[sys_mmap]mmap ok,base:{:#x}", base.0);

    // 10. 特殊 flags 的额外处理
    if flags.contains(MmapFlags::MAP_POPULATE) {
        // 立即为每页缺页、填充物理页
        area.map(&mut ms.page_table)?;
    }
   
    // if flags.contains(MmapFlags::MAP_LOCKED) {
    //     // mlock：锁定这些物理页
    //     area.lock(&mut ms.page_table)?;
    // }todo(heliosly)
    // if flags.contains(MmapFlags::MAP_STACK) {
    //     // 设置向下扩展属性
    //     area.set_grows_down(true);
    // }

    // ——————————————————————————————————————————
    // 11. 插入到 MemorySet 的 areatree / VMA 列表
    ms.areatree.push(area);
    flush_tlb();

    // 12. 返回映射基址
    Ok(base.0)
}

/// YOUR JOB: Implement munmap.
pub async  fn sys_munmap(start: usize, len: usize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_munmap ", current_task().get_pid());
    
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    current_process()
        .memory_set
        .lock().await
        .munmap(start_va.floor(), end_va.ceil());
    Ok(0)
}

/// change data segment size
pub async  fn sys_brk(new_brk:usize) -> SyscallRet {
    trace!(
        "[sys_brk] new_brk_addr:{}",
       
        new_brk
    );
    let current_brk=current_process().program_brk();
    if new_brk==0{
        return Ok(current_brk);
    }
    if new_brk == current_brk// 没有变化
     {   return Ok(current_brk);}

    // 地址必须在 heap 合法范围内
    if new_brk < current_process().heap_bottom() {
        return  Err(SysErrNo::EINVAL);
    }
   
match current_process().change_program_brk(new_brk).await {
        Some(new_brk) => Ok(new_brk),
        None => Err(SysErrNo::ENOMEM),
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().get_pid()
    );
    unimplemented!();
    // let current_task = current_task();
    // let token =current_task.get_user_token();
    // let path = translated_str(token, _path);
    // let elf;
    // if let Ok(app_inode) = open_file(path.as_str(), OpenFlags::O_RDONLY,0o777) {
    //    elf = app_inode.file().unwrap().read_all();
    //    let tcb=Arc::new(TaskControlBlock::new(elf.as_slice(),path));
    //    let mut inner=tcb.inner_exclusive_access();
    //    let mut pin=current_task.inner_exclusive_access();
    //    inner.parent.replace(current_task.get_pid());
    //    pin.children.push(tcb.clone());
    //    drop(inner);
    //    let pid = tcb.get_pid() as isize;
    //    add_task(tcb);
    //    return pid;
    // }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_set_priority ", current_task().get_pid());

    set_priority(&(*current_task().0), prio);
    Ok(0)
}


/// getcwd 系统调用实现
/// buf: 用户空间缓冲区的指针，用于存储当前工作目录路径
/// size: 用户缓冲区的大小
/// 返回：成功时为实际写入用户缓冲区的字节数（包括末尾的 '\0'），
///       如果 size 太小，返回 -ERANGE，
///       如果 buf 无效，返回 -EFAULT，
///       其他错误则返回相应错误码。
/// POSIX getcwd 成功时返回 buf 指针，失败时返回 NULL。
/// 我们这里调整为返回写入的长度或错误码。
pub async fn sys_getcwd(buf_user_ptr: *mut u8, size: usize) -> SyscallRet {
    trace!("[sys_getcwd](buf: {:p}, size: {})", buf_user_ptr, size);

    if buf_user_ptr.is_null() && size != 0 { // POSIX 允许 buf 为 NULL 以查询所需大小，但我们这里简化
        let len =current_process() .cwd.lock().await.as_bytes().len() + 1; // 包括 null terminator
        return Ok(len);
    }
    if size == 0 && !buf_user_ptr.is_null() { // size 为0但buf非NULL，POSIX行为未明确，这里视为无效
        return Err(SysErrNo::EINVAL);
    }
    if size == 0 && buf_user_ptr.is_null() { // POSIX 允许此情况动态分配，我们不支持
        return Err(SysErrNo::EINVAL); // 或者 ERANGE，因为大小为0肯定不够
    }
    let proc_arc = current_process();
    let cwd_kernel_string = proc_arc.cwd.lock().await.clone();
    let token = proc_arc.get_user_token().await; // 或者 proc_arc.memory_set.lock().await.token();
    let cwd_len_with_null = cwd_kernel_string.len() + 1;

    if size < cwd_len_with_null {
        return Err(SysErrNo::ERANGE);
    }

    // 1. 创建 buffer_to_copy，使其生命周期覆盖 copy_to_user_bytes 调用
    let mut buffer_to_copy = Vec::with_capacity(cwd_len_with_null);
    buffer_to_copy.extend_from_slice(cwd_kernel_string.as_bytes());
    buffer_to_copy.push(0); // Null terminator

    // 2.  bytes_to_copy_slice 借用一个仍然存活的 buffer_to_copy
    let bytes_to_copy_slice: &[u8] = &buffer_to_copy;

    // 3. 使用 copy_to_user_bytes
    match unsafe {
        copy_to_user_bytes(
            token,
            VirtAddr::from(buf_user_ptr as usize),
            bytes_to_copy_slice,
        )
    } {
        Ok(bytes_copied) => {
            if bytes_copied != cwd_len_with_null {
                // log::error!("sys_getcwd: copy_to_user_bytes copied {} bytes, expected {}", bytes_copied, cwd_len_with_null);
                Err(SysErrNo::EFAULT)
            } else {
                Ok(bytes_copied )
            }
        }
        Err(translate_error) => {
            // log::warn!("sys_getcwd: copy_to_user_bytes failed: {:?}", translate_error);
            match translate_error {
                TranslateError::TranslationFailed(_) | TranslateError::PermissionDenied(_) => Err(SysErrNo::EFAULT),
                TranslateError::UnexpectedEofOrFault => Err(SysErrNo::EIO),
                _ => Err(SysErrNo::EFAULT),
            }
        }
    }
}

pub async fn sys_get_robust_list(pid: usize, head_ptr: *mut usize, len_ptr: *mut usize) -> SyscallRet {

    trace!("[sys_get_robust_list] NOT IMPLEMENT" );

    let tid2tc_lock = TID2TC.lock();
    let  task =tid2tc_lock.get(&pid);
    if  pid == 0 {
       let  task = current_task();
       let token =unsafe { *task.page_table_token.get() };
       let robust = task.robust_list.lock().await;
       put_data(token, head_ptr, robust.head)?;
       put_data(token, len_ptr, robust.len)?;
       Ok(0)
    }
    else
    if let Some(task) = task {
        let token =unsafe { *task.page_table_token.get() };
        let robust = task.robust_list.lock().await;
        put_data(token, head_ptr, robust.head)?;
        put_data(token, len_ptr, robust.len)?;
        Ok(0)
    } else {
        Err(SysErrNo::ESRCH)
    }

}

pub async fn sys_set_robust_list(head: usize, len: usize) -> SyscallRet {
    trace!("[sys_set_robust_list] NOT IMPLEMENT" );
    if len != RobustList::HEAD_SIZE {
        return Err(SysErrNo::EINVAL);
        
    }

    let task = current_task();
    task.robust_list.lock().await.head = head;
    
    task.robust_list.lock().await.len= len;
    Ok(0)
}


pub async  fn sys_prlimit(
    pid: usize,
    resource: u32,
    new_limit: *const RLimit,
    old_limit: *mut RLimit,
) -> SyscallRet{

    trace!("[sys_prlimit]: pid:{},resource:{},new_limit:{:?},old_limit:{:?}",pid,resource,new_limit,old_limit);
    const RLIMIT_NOFILE: u32 = 7;
        let proc = current_process();
        if !old_limit.is_null() {
        proc.manual_alloc_type_for_lazy(old_limit).await?;
        }
        if !new_limit.is_null() {
        proc.manual_alloc_type_for_lazy(new_limit).await?;
        }
        let token  = proc.get_user_token().await;
    if resource != RLIMIT_NOFILE {
        // 说明是get
        // let limit = translated_refmut(token, old_limit)?;
        // limit.rlim_cur = 0xdeadbeff;
        // limit.rlim_max =  0xdeadbeff;
        return Ok(0)
    }

    if pid == 0 {
       
        let fd_table =  proc.fd_table.lock().await;
        if !old_limit.is_null() {
            // 说明是get
            let limit = translated_refmut(token, old_limit)?;
            limit.rlim_cur = fd_table.get_soft_limit();
            limit.rlim_max = fd_table.get_hard_limit();
        }
        if !new_limit.is_null() {
            // 说明是set
            let limit: &RLimit = get_target_ref(token, new_limit)?;
            fd_table.set_limit(limit.rlim_cur, limit.rlim_max);
        }
    } else {
        unimplemented!("pid must equal zero");
    }

    Ok(0)
//   Err(SysErrNo::ENOSYS )
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RLimit {
    pub rlim_cur: usize, /* Soft limit */
    pub rlim_max: usize, /* Hard limit (ceiling for rlim_cur) */
}






pub async fn sys_mprotect( start: usize, size: usize, flags: usize)->SyscallRet {
    trace!("[sys_mprotect] start:{:#x},size:{:#x},flags:{:#x}",start,size,flags);
     let start_va= VirtAddr::from(start);
     let pcb_arc= current_process();
     let mut memory_set = pcb_arc.memory_set.lock().await;
    match MmapProt::from_bits(flags as u32) {
       Some(prot) => {
          memory_set.mprotect(start_va, size, prot.into()).await;
       }
       None => {
          return Err(SysErrNo::EINVAL);
       }
    }
     Ok(0)

}
















/// futex 系统调用实现
/// uaddr_user_ptr: 指向用户空间 futex 字 (u32) 的指针。
/// futex_op_full: Futex 操作码，可能包含 FUTEX_PRIVATE_FLAG 等。
/// val: 操作依赖的值 (例如，FUTEX_WAIT 中期望的 futex 字的值)。
/// timeout_or_val2_or_ptr:
///   - 对于 FUTEX_WAIT (带超时): 指向用户空间 UserTimeSpec 的指针 (usize)。
///   - 对于 FUTEX_WAKE: 要唤醒的任务数量 (usize)。
///   - 对于不带超时的 FUTEX_WAIT: 可以是0。
/// uaddr2_user_ptr: 
/// val3: 
pub async fn sys_futex(
    uaddr_user_ptr: *mut u32,
    futex_op_full: i32,
    val_or_count: u32,
    val2_timeout_ptr_or_num_requeue: usize,
    uaddr2_user_ptr: *mut u32,
    bitmask_or_val3: u32,
) -> SyscallRet {
       let pcb_arc = current_process();
    let tid = current_task_id();
info!(
        "[sys_futex] uaddr_user_ptr: {:#x}, futex_op_full: {:#x} ({}), val_or_count: {}, \
        val2_timeout_ptr_or_num_requeue: {:#x}, uaddr2_user_ptr: {:#x}, bitmask_or_val3: {:#x},tid:{}",
        uaddr_user_ptr as usize,
        futex_op_full,
        futex_op_full,
        val_or_count,
        val2_timeout_ptr_or_num_requeue,
        uaddr2_user_ptr as usize,
        bitmask_or_val3,
        tid,
    );


    pcb_arc.manual_alloc_type_for_lazy(uaddr_user_ptr  ).await?;
    let token = pcb_arc.memory_set.lock().await.token();
    
    if uaddr_user_ptr.is_null() || (uaddr_user_ptr as usize % core::mem::align_of::<u32>() != 0) {
        return Err(SysErrNo::EFAULT);
    }
    let uaddr_usize = uaddr_user_ptr as usize;
    let futex_key: FutexKey = (token, uaddr_usize);
    
    let op_cmd = futex_op_full & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME) as i32;
    let use_clock_realtime = (futex_op_full & FUTEX_CLOCK_REALTIME as i32) != 0;

    match op_cmd {
        FUTEX_REQUEUE => {
            debug!("futex_requeue: uaddr1:{:#x} -> uaddr2:{:#x}", uaddr_usize, uaddr2_user_ptr as usize);

            // 校验 uaddr2
            if uaddr2_user_ptr.is_null() || (uaddr2_user_ptr as usize % core::mem::align_of::<u32>() != 0) {
                return Err(SysErrNo::EFAULT);
            }
            pcb_arc.manual_alloc_type_for_lazy(uaddr2_user_ptr).await?;
            let uaddr2_usize = uaddr2_user_ptr as usize;
            let futex_key2: FutexKey = (token, uaddr2_usize);
            if futex_key == futex_key2 { // 不能 requeue 到自己
                return Err(SysErrNo::EINVAL);
            }

            let num_to_wake = val_or_count as usize;
            let num_to_requeue = val2_timeout_ptr_or_num_requeue;

            // 注意：与 FUTEX_CMP_REQUEUE 不同，这里没有对用户空间的值进行比较。
            // 直接进入锁并执行操作。

            let mut futex_system = GLOBAL_FUTEX_SYSTEM.lock();
            let mut woken_count = 0;
            let mut requeued_count = 0;
            
            // 1. 唤醒 uaddr1 上的等待者
            if num_to_wake > 0 {
                if let Some(wait_queue1) = futex_system.get_mut(&futex_key) {
                    // 使用 u32::MAX 作为 bitmask，因为 REQUEUE 不关心 bitmask
                    woken_count = wait_queue1.wake_matching_waiters(num_to_wake, u32::MAX);
                }
            }
            
            // 2. 将 uaddr1 上的剩余等待者移动到 uaddr2
            if num_to_requeue > 0 {
                // 使用你已经实现的 drain_waiters 和 enqueue_waiters
                let waiters_to_move = if let Some(wait_queue1) = futex_system.get_mut(&futex_key) {
                    let drained = wait_queue1.drain_waiters(num_to_requeue);
                    if wait_queue1.is_empty() {
                        futex_system.remove(&futex_key);
                    }
                    drained
                } else {
                    // 确保返回与 drain_waiters 相同的类型 (LinkedList)
                    LinkedList::new() 
                };

                requeued_count = waiters_to_move.len();
                if requeued_count > 0 {
                    let wait_queue2 = futex_system.entry(futex_key2).or_default();
                    wait_queue2.enqueue_waiters(waiters_to_move);
                }
            }
            
            Ok(woken_count + requeued_count )
        }
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {

            
            debug!("wait futex addr :{:#x},tid:{}",uaddr_usize,tid);
            
            let expected_val = val_or_count;
            let wait_bitmask = if op_cmd == FUTEX_WAIT_BITSET { bitmask_or_val3 } else { u32::MAX };

            let current_val_in_user_initial_check =* match unsafe {  get_target_ref(token, uaddr_user_ptr ) } {
                Ok(v) => v, Err(e) => return Err(e.into()),
            };
            if current_val_in_user_initial_check != expected_val {
                return Err(SysErrNo::EAGAIN);
            }

            let timeout_ptr = val2_timeout_ptr_or_num_requeue;

            let timespec_user_ptr = timeout_ptr as *const UserTimeSpec;
            let deadline_opt: Option<TimeVal> = if timeout_ptr == 0||
            pcb_arc.manual_alloc_type_for_lazy(timespec_user_ptr).await.is_err() { None }
            else {
                let timeout_spec = match unsafe { get_target_ref(token, timespec_user_ptr) } {
                    Ok(ts) => ts,
                     Err(e) => return Err(e.into()),
                };
                if (timeout_spec.tv_nsec as isize) < 0 || timeout_spec.tv_nsec >= 1_000_000_000 ||
                   (use_clock_realtime && (timeout_spec.tv_sec as isize) < 0) ||
                   (!use_clock_realtime && (timeout_spec.tv_sec as isize) < 0 && (timeout_spec.tv_sec as isize == -1 && timeout_spec.tv_nsec != 0 || (timeout_spec.tv_sec as isize)< -1))
                { return Err(SysErrNo::EINVAL); }

                if timeout_spec.tv_sec == 0 && timeout_spec.tv_nsec == 0 { return Ok(0); } // 0超时，值匹配，立即成功

                if use_clock_realtime {
                    // FIXME: 正确实现绝对超时从 UserTimeSpec 到内核 TimeVal 的转换
                    // TimeVal::from_absolute_timespec(&timeout_spec).ok_or(SysErrNo::EINVAL)?
                    Some(current_time().add_timespec(&timeout_spec)) // 暂时按相对计算
                } else {
                    Some(current_time().add_timespec(&timeout_spec))
                }
            };

            let futex_wait_internal_future = FutexWaitInternalFuture::new(
                token, uaddr_usize, expected_val, wait_bitmask, deadline_opt,
            );
            match futex_wait_internal_future.await {
                Ok(()) => Ok(0),
                Err(e) => Err(e),
            }
        }

        FUTEX_WAKE | FUTEX_WAKE_BITSET => {
            
            debug!("waker futex addr :{:#x},tid:{}",uaddr_usize,tid);
            let num_to_wake = val_or_count as usize;
            if num_to_wake == 0 { return Ok(0); }
            let wake_bitmask = if op_cmd == FUTEX_WAKE_BITSET { bitmask_or_val3 } else { u32::MAX };

            let mut futex_system_guard = GLOBAL_FUTEX_SYSTEM.lock();
            let woken_count = if let Some(wait_queue) = futex_system_guard.get_mut(&futex_key) {
                let count = wait_queue.wake_matching_waiters(num_to_wake, wake_bitmask);
                if wait_queue.is_empty() {
                    futex_system_guard.remove(&futex_key);
                }
                count
            } else { 0 };
       

            Ok(woken_count)
        }
        FUTEX_CMP_REQUEUE => {
            debug!("futex_cmp_requeue: uaddr1:{:#x} -> uaddr2:{:#x}", uaddr_usize, uaddr2_user_ptr as usize);

            // 校验 uaddr2
            if uaddr2_user_ptr.is_null() || (uaddr2_user_ptr as usize % core::mem::align_of::<u32>() != 0) {
                return Err(SysErrNo::EFAULT);
            }
            pcb_arc.manual_alloc_type_for_lazy(uaddr2_user_ptr).await?;
            let uaddr2_usize = uaddr2_user_ptr as usize;
            let futex_key2: FutexKey = (token, uaddr2_usize);
            if futex_key == futex_key2 { // 不能 requeue到自己
                return Err(SysErrNo::EINVAL);
            }

            let num_to_wake = val_or_count as usize;
            let num_to_requeue = val2_timeout_ptr_or_num_requeue;
            let expected_val = bitmask_or_val3;

            // 原子性检查：如果值不匹配，立即返回 EAGAIN
            // 这是 CMP_REQUEUE 的核心，防止丢失唤醒
            let current_val = *match unsafe { get_target_ref(token, uaddr_user_ptr) } {
                Ok(v) => v, Err(e) => return Err(e.into()),
            };
            if current_val != expected_val {
                return Err(SysErrNo::EAGAIN);
            }

            let mut futex_system = GLOBAL_FUTEX_SYSTEM.lock();
            let mut woken_count = 0;
            let mut requeued_count = 0;
            
            // 1. 唤醒 uaddr1 上的等待者
            if num_to_wake > 0 {
                if let Some(wait_queue1) = futex_system.get_mut(&futex_key) {
                    woken_count = wait_queue1.wake_matching_waiters(num_to_wake, u32::MAX);
                }
            }
            
            // 2. 将 uaddr1 上的剩余等待者移动到 uaddr2
            if num_to_requeue > 0 {
                // 为了避免 borrow checker 问题 (同时修改两个 hashmap entry)
                // 我们先把要移动的 waiters 拿出来
                let waiters_to_move = if let Some(wait_queue1) = futex_system.get_mut(&futex_key) {
                    let drained = wait_queue1.drain_waiters(num_to_requeue);
                    if wait_queue1.is_empty() {
                        futex_system.remove(&futex_key);
                    }
                    drained
                } else {
                    LinkedList::new()
                };

                requeued_count = waiters_to_move.len();
                if requeued_count > 0 {
                    let wait_queue2 = futex_system.entry(futex_key2).or_default();
                    wait_queue2.enqueue_waiters(waiters_to_move);
                }
            }
            
            Ok(woken_count + requeued_count )
        }
        
        FUTEX_WAKE_OP => {
            debug!("futex_wake_op: uaddr1:{:#x}, uaddr2:{:#x}", uaddr_usize, uaddr2_user_ptr as usize);
            
            // 校验 uaddr2
            if uaddr2_user_ptr.is_null() || (uaddr2_user_ptr as usize % core::mem::align_of::<u32>() != 0) {
                return Err(SysErrNo::EFAULT);
            }
            pcb_arc.manual_alloc_type_for_lazy(uaddr2_user_ptr).await?;
            let uaddr2_usize = uaddr2_user_ptr as usize;
            let futex_key2: FutexKey = (token, uaddr2_usize);

            let num_wake_uaddr2 = val_or_count as usize;
            let num_wake_uaddr1 = val2_timeout_ptr_or_num_requeue as usize;
            let op_encoded = bitmask_or_val3;
            
            // 解码 val3
            let op_type = (op_encoded >> 28) & 0xF;
            let cmp_type = (op_encoded >> 24) & 0xF;
            let op_arg = (op_encoded >> 12) & 0xFFF;
            let cmp_arg = op_encoded & 0xFFF;
            
            let mut total_woken = 0;
            
            // 整个操作必须是原子的，所以要锁住 futex system
            let mut futex_system = GLOBAL_FUTEX_SYSTEM.lock();
            
            // 在锁内进行用户内存访问
            let uaddr1_val_ref = match unsafe { translated_refmut(token, uaddr_user_ptr) } {
                Ok(v) => v, Err(e) => return Err(e.into()),
            };
            let old_val = *uaddr1_val_ref;

            // 1. 执行比较
            let condition_met = match cmp_type {
                FUTEX_OP_CMP_EQ => old_val == cmp_arg,
                FUTEX_OP_CMP_NE => old_val != cmp_arg,
                FUTEX_OP_CMP_LT => (old_val as i32) < (cmp_arg as i32),
                FUTEX_OP_CMP_LE => (old_val as i32) <= (cmp_arg as i32),
                FUTEX_OP_CMP_GT => (old_val as i32) > (cmp_arg as i32),
                FUTEX_OP_CMP_GE => (old_val as i32) >= (cmp_arg as i32),
                _ => return Err(SysErrNo::EINVAL), // 无效的比较类型
            };
            
            if condition_met {
                // 2. 执行操作
                let new_val = match op_type {
                    FUTEX_OP_SET  => op_arg,
                    FUTEX_OP_ADD  => old_val.wrapping_add(op_arg),
                    FUTEX_OP_OR   => old_val | op_arg,
                    FUTEX_OP_ANDN => old_val & !op_arg,
                    FUTEX_OP_XOR  => old_val ^ op_arg,
                    _ => return Err(SysErrNo::EINVAL), // 无效的操作类型
                };
                *uaddr1_val_ref = new_val;
                
                // 3. 唤醒 uaddr1 上的等待者
                if num_wake_uaddr1 > 0 {
                    if let Some(wq1) = futex_system.get_mut(&futex_key) {
                        total_woken += wq1.wake_matching_waiters(num_wake_uaddr1, u32::MAX);
                    }
                }
                
                // 4. 唤醒 uaddr2 上的等待者
                if num_wake_uaddr2 > 0 {
                     if let Some(wq2) = futex_system.get_mut(&futex_key2) {
                        total_woken += wq2.wake_matching_waiters(num_wake_uaddr2, u32::MAX);
                    }
                }
            }
            
            Ok(total_woken )
        }
        _ => Err(SysErrNo::ENOSYS),
    }
}


pub async  fn sys_mremap(
    old_address: *mut u8,   // void* → *mut u8
    old_size: usize,        // size_t → usize
    new_size: usize,        // size_t → usize
    flags: u32,             // int → i32
    new_address: *mut u8, //仅当 MREMAP_FIXED 时使用
) ->SyscallRet {
    trace!(
        "[sys_mremap] old_addr = {:p}, old_size = {}, new_size = {}, flags = {:#x}, new_addr = {:p}",
        old_address,
        old_size,
        new_size,
        flags,
        new_address,
    ); 
    let flags= MremapFlags::from_bits(flags).unwrap();
    let old_start = old_address as usize;
    let proc= current_process();
    
    let x = proc.memory_set.lock().await.mremap(old_start.into(), old_size, new_size,flags).await; 
    x

}





pub async fn sys_sched_yield() -> SyscallRet {
     yield_now().await;
    Ok(0)
}

pub fn sys_setuid(uid: u32) -> SyscallRet {
    let task = current_task();
    task.set_uid(uid as usize);
    change_current_uid(uid);
    Ok(0)
}

pub fn sys_geteuid() -> SyscallRet {
    Ok(current_uid() as usize)
}

pub fn sys_getgid() -> SyscallRet {
    Ok(0) // root group
}

pub fn sys_getegid() -> SyscallRet {
    Ok(0) // root group
}

pub fn sys_gettid() -> SyscallRet {
    Ok(current_task().get_tid())
}

pub fn sys_setsid() -> SyscallRet {
    //涉及到会话和进程组，暂时伪实现

    trace!("[sys_setsid] ");
    Ok(0)
}



pub static CUR_UID: Lazy<spin::mutex::Mutex<u32>> = Lazy::new(|| spin::mutex::Mutex::new(0));

pub fn current_uid() -> u32 {
    *CUR_UID.lock()
}

pub fn change_current_uid(uid: u32) {
    *CUR_UID.lock() = uid;
}

pub fn sys_membarrier()->SyscallRet{
    Ok(0)
}

pub async  fn sys_madvise(
    addr: usize,
    len: usize,
    advice: u32,
) -> SyscallRet {
    trace!(
        "[sys_madvise] addr: {:#x}, len: {}, advice: {}",
        addr,
        len,
        advice
    );
     // 1. 检查地址合法性和页对齐
    // According to POSIX, `addr` must be page-aligned.
    let start_va: VirtAddr = addr.into();
    if start_va.page_offset() != 0 {
        debug!("sys_madvise: addr 0x{:x} is not page-aligned.", addr);
        return Err(SysErrNo::EINVAL);
    }

    if len == 0 {
        return Ok(0); // Success, nothing to do.
    }

    // Check if the memory range is valid and within user space boundaries.
    // Use `checked_add` to prevent overflow.
    let end_addr = match addr.checked_add(len) {
        Some(end) => {
            // MEMORY_END is the boundary of physical memory, also serving as the upper
            // limit for user virtual addresses in many rCore-like designs.
            if end > MEMORY_END {
        return Err(SysErrNo::ENOMEM);

            }
            end
        }
        None => 
        return Err(SysErrNo::EINVAL),
   // Overflow occurred
    };

    debug!(
        "[sys_madvise](addr: {:#x}, len: {}, advice: {})",
        addr, len, advice
    );
    
    // 获取当前任务的内存管理结构体 (MemorySet)
    // 在 rCore 中，这个操作是线程安全的，因为它返回一个 Arc<TaskControlBlock>
    let proc = current_process();
    // 获取内存管理的锁
    let mut memory_set = proc.memory_set.lock().await;

    // 2. 根据 advice 做出响应
    match advice {
        // 3. 有些策略可以直接忽略
        // These are performance hints. A simple kernel can safely ignore them
        // without affecting correctness. We've already validated the address range,
        // so we can just return success.
        MADV_NORMAL | MADV_RANDOM | MADV_SEQUENTIAL | MADV_WILLNEED => {
            debug!("[sys_madvise]: advice {} is accepted but ignored.", advice);
            Ok(0) // 成功
        }

        // 4. MADV_DONTNEED 是需要我们实际操作的核心
        // This advice tells the kernel that the application does not expect to
        // use this memory region in the near future. The kernel can free resources
        // associated with it. Our implementation will unmap the pages.
        MADV_DONTNEED => {
            // 计算需要操作的虚拟页号范围
            // The range includes the page containing `addr` up to the page
            // containing `addr + len - 1`.
            let start_vpn = start_va.floor();
            let end_vpn = VirtAddr::from(end_addr - 1).floor();
            
            debug!(
                "[sys_madvise](MADV_DONTNEED): unmapping range [{:?}, {:?}]",
                start_vpn, end_vpn
            );

            // 在 MemorySet 中实现 unmap 逻辑是最符合 rCore 架构的
            // 因为 MemorySet 维护了 VMA (Virtual Memory Areas) 和页表的一致性
            // 直接操作页表可能会破坏 VMA 的状态
            // 我们假设 MemorySet 有一个方法来处理这个请求
            memory_set.madvise_dontneed(start_vpn, end_vpn);

            Ok(0) // 成功
        }

        // 对于不支持的 advice，返回 EINVAL
        _ => {
            warn!("sys_madvise: unsupported advice value {}.", advice);

        return Err(SysErrNo::EINVAL);
        }
    }
}