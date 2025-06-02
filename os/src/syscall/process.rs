//! Process management syscalls
//!


use alloc::{
   format, string::{String, ToString}, sync::Arc, vec::Vec
};
use linux_raw_sys::general::AT_FDCWD;

use crate::{
    config::{MAX_SYSCALL_NUM, MMAP_TOP, PAGE_SIZE},
    fs::{open_file,  OpenFlags, NONE_MODE},
    mm::{
        flush_tlb, get_target_ref, page_table::copy_to_user_bytes, put_data, translated_byte_buffer, translated_refmut, translated_str, MapArea, MapAreaType, MapPermission, MapType, MmapFile, MmapFlags, TranslateRefError, VirtAddr, VirtPageNum
    },
    syscall::flags:: MmapProt,
    task::{
        current_process, current_task, current_token, exit_current_and_run_next, set_priority, yield_now, CloneFlags, ProcessRef, RobustList, TaskStatus, PID2PC, TID2TC
    },
    timer::{ get_time_us, get_usertime, TimeVal, UserTimeSpec},
    utils::{
        error::{SysErrNo, SyscallRet}, page_round_up, string::get_abs_path
    },
};

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
    trace!("[sys_getppid] pid :{}", current_task().get_pid());
    Ok(current_process().parent())
}
pub async  fn sys_exit(exit_code: i32) -> SyscallRet {
    trace!("kernel:pid[{}] sys_exit", current_task().get_pid());

    exit_current_and_run_next(exit_code).await;
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
    debug!(
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

    proc.clone_task(flags, user_stack, ptid, tls, ctid).await
}


pub  async  fn sys_execve(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> SyscallRet {
    
    let process = current_process();
    
    let token = process.get_user_token().await;
    let mut path = translated_str(token, path);
    path = process.resolve_path_from_fd(AT_FDCWD , path.as_str(),true).await?;
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
    if path.ends_with(".sh") {
        //.sh文件不是可执行文件，需要用busybox的sh来启动
        argv_vec.insert(0, String::from("sh"));
        argv_vec.insert(0, String::from("busybox "));
        path = String::from("/musl/busybox");
    }
    
    
    // if path.ends_with("ls") || path.ends_with("xargs") || path.ends_with("sleep") {
    //     //ls,xargs,sleep文件为busybox调用，需要用busybox来启动
    //     argv_vec.insert(0, String::from("busybox"));
    //     path = String::from("/busybox");
    // }

    // println!("[sys_execve] path is {},arg is {:?}", path, argv_vec);
    debug!("[sys_execve] path is {},arg is {:?}", path, argv_vec);
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
        tcb.clear_child_tid().unwrap();
    }

    process.set_exe(abs_path).await;
    process.exec(&elf_data, &argv_vec, &mut env).await?;
    
    process.memory_set.lock().await.activate();
    Ok(0)
}

pub fn sys_settidaddress(tid_ptr:usize) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_settidaddress NOT IMPLEMENTED",
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
//  参考 https://man7.org/linux/man-pages/man2/wait4.2.html
pub async  fn sys_wait4(pid: isize, wstatus: *mut i32, options: i32) -> SyscallRet {
    trace!("[sys_wait4] pid:{},wstatus:{:?},options:{}",pid,wstatus,options);
    let proc = current_process();
    
    if (pid as i32) == i32::MIN {
        return Err(SysErrNo::ESRCH);
    }
    if options < 0 || options > 100 {
        return Err(SysErrNo::EINVAL);
    }
    let mut childvec = proc.children.lock().await;
    if !childvec
        .iter()
        .any(|p| pid == -1 || pid as usize == p.get_pid())
    {
        debug!(
            "cant find pid:{} in parent pid:{},and children count = : {} ",
            pid,
            proc.get_pid(),
            childvec.len()
        );
        return Err(SysErrNo::ECHILD);
        // ---- release current PCB
    }
    let mut pair = None;
    for (idx, p) in childvec.iter().enumerate() {
        if p.is_zombie().await && (pid == -1 || pid as usize == p.get_pid()) {
            pair = Some((idx, p));
            break;
        }
    }
    if let Some((idx, child_task)) = pair {
        debug!("chiled pid is :{} removed",child_task.get_pid());
        
        PID2PC.lock().remove(&child_task.get_pid());
        let child = childvec.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        let found_pid = child.get_pid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.exit_code();

        // ++++ release child PCB
        if wstatus as usize != 0x0 {
            debug!(
                "[sys_wait4] wait pid {}: child {} exit with code {}, wstatus= {:#x}",
                pid, found_pid, exit_code, wstatus as usize
            );
            let mut ms = proc.memory_set.lock().await;
            if exit_code >= 128 && exit_code <= 255 {
                //表示由于信号而退出的
               ms. safe_put_data( wstatus, exit_code).await.unwrap();
            } else {
                ms.safe_put_data( wstatus, exit_code << 8).await.unwrap();
            }
        }
        assert_eq!(
            Arc::strong_count(&child),
            1,
            "strong_count is incorrect,{}",
            Arc::strong_count(&child)
        );
        Ok(found_pid)
    } else {

        return Err(SysErrNo::EAGAIN);
    }
    // ---- release current PCB automatically
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
    trace!(
        "kernel:pid[{}] sys_exit_exitgroup NOT IMPLEMENTED",
        current_task().get_pid()
    );
    exit_current_and_run_next(exit_code).await;
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
    fd: usize,
    off: usize,
) -> SyscallRet {
    //需要有严格的判断返回错误的顺序！！！
    // 0. flags 不能全 0
    if flags == 0 {
        return Err(SysErrNo::EINVAL);
    }
    let flags = match MmapFlags::from_bits(flags) {
        Some(f) => f,
        None => return Err(SysErrNo::EINVAL),
    };

    // 1. 长度不能为 0
    if len == 0 {
        return Err(SysErrNo::EINVAL);
    }
    // 2. 偏移量必须页对齐
    if off % PAGE_SIZE != 0 {
        return Err(SysErrNo::EINVAL);
    }

    // 3. 匿名映射 | 文件映射：fd 检查
    let anon = flags.contains(MmapFlags::MAP_ANONYMOUS);
    if !anon {
        // 非匿名必须提供合法 fd
        if fd == usize::MAX {
            return Err(SysErrNo::EBADE);
        }
    }
    // 4. prot -> MapPermission
    let mmap_prot = match MmapProt::from_bits(prot) {
        Some(f) => f,
        None => return Err(SysErrNo::EINVAL),
    };
    let map_perm: MapPermission = mmap_prot.into();
    debug!(
        "[sys_mmap]: addr {:#x}, len {:#x}, fd {}, offset {:#x}, flags {:?}, prot is {:?}, map_perm {:?}",
        addr, len, fd as isize, off, flags,mmap_prot, map_perm
    );
    
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
        match ms.areatree.alloc_pages(pages) {
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

    debug!("[sys_mmap]mmap ok,base:{:#x}", base.0);

    // 10. 特殊 flags 的额外处理
    if flags.contains(MmapFlags::MAP_POPULATE) {
        // 立即为每页缺页、填充物理页
        area.map(&mut ms.page_table);
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
pub fn sys_geteuid() -> SyscallRet {
   
        trace!(
            "kernel:pid[{}] sys_getuid NOT IMPLEMENTED",
            current_task().get_pid()
        );
        //todo(heliosly)
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
                TranslateRefError::TranslationFailed(_) | TranslateRefError::PermissionDenied(_) => Err(SysErrNo::EFAULT),
                TranslateRefError::UnexpectedEofOrFault => Err(SysErrNo::EIO),
                _ => Err(SysErrNo::EFAULT),
            }
        }
    }
}
pub fn sys_gettid() -> SyscallRet {
    trace!("kernel:pid[{}] sys_gettid ", current_task().get_pid());
 
    Ok(current_task().id.as_usize())
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
        let limit = translated_refmut(token, old_limit)?;
        limit.rlim_cur = 0xdeadbeff;
        limit.rlim_max =  0xdeadbeff;
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