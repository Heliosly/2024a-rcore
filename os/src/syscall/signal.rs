// use crate::{signal::signal::{SigAction, SigSet}, utils::error::{SysErrNo, SyscallRet}};


// pub fn sys_sigprocmask(how: usize, set: *const SigSet, oldset: *mut SigSet) -> SyscallRet {
//     if how > 2 {
//         return Err( SysErrNo::EINVAL);
//     }
//     // 正常执行逻辑
//     Ok(0)
// }

use alloc::vec::Vec;

use crate::{mm::{get_target_ref, put_data, translated_refmut}, signal::{load_trap_for_signal, send_signal_to_task, SigAction, SigMaskHow, SigSet, Signal, NSIG}, task::{current_process, current_task, PID2PC, TID2TC}, utils::error::{SysErrNo, SyscallRet}};

// pub fn sys_rt_sigaction(
//     signo: usize,
//     act: *const SigAction,
//     old_act: *mut SigAction,
// ) -> SyscallRet {
// Ok(0)
// }
// 通常由 trampoline 调用，用于从信号处理函数返回
pub async  fn sys_sigreturn()-> SyscallRet {
 
    trace!("[sys_sigreturn]");
    if load_trap_for_signal().await {
        // 说明确实存在着信号处理函数的trap上下文
        // 此时内核栈上存储的是调用信号处理前的trap上下文
        Ok(current_task().get_trap_cx().unwrap().get_ret_code() )
        
    } else {
        // 没有进行信号处理，但是调用了sig_return
        // 此时直接返回EPERM
        Err(SysErrNo::EPERM)
    }
 
}




// int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
pub async fn sys_sigaction(
    signum_usize: usize,
    act_user_ptr: *const SigAction,
    oldact_user_ptr: *mut SigAction,
) -> SyscallRet {

    trace!("[sys_sigaction] signo: {}, act: {:?}, oldact: {:?}", signum_usize, act_user_ptr, oldact_user_ptr);
    let sig = match Signal::from_usize(signum_usize) {
        Some(s) => s,
        None => return Err(SysErrNo::EINVAL), // 无效信号
    };
    let process = current_process();
    let token =process.get_user_token().await;
    // SIGKILL 和 SIGSTOP 的动作不能被改
    if sig == Signal::SIGKILL || sig == Signal::SIGSTOP {
        return Err(SysErrNo::EINVAL);
    }

    let mut shared_state = process.signal_shared_state.lock().await;

    // 如果 oldact 非空，保存旧的动作
    if !oldact_user_ptr.is_null() {
       
       process.memory_set.lock().await .safe_put_data( oldact_user_ptr,  shared_state.sigactions[sig as usize]).await? ;
          
    }

    if !act_user_ptr.is_null() {
        
         let new_action=get_target_ref(token, act_user_ptr)?;

        // 校验 new_action 的合法性 (例如 handler 地址)
        // ...

        shared_state.sigactions[sig as usize] = *new_action;
        // log::debug!("Task {} set action for signal {:?} to handler 0x{:x}", current_task_arc.id(), sig, new_action.handler);
    }

   
    Ok(0) // 成功
}

// int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
pub async  fn sys_sigprocmask(
    how: i32,
    set_user_ptr: *const SigSet,
    oldset_user_ptr: *mut SigSet,
) -> SyscallRet {
    trace!("[sys_sisgprocmask]");
    let current_task_arc = current_task();
    let mut signal_state = current_task_arc.signal_state.lock().await;
    let token = current_task_arc.get_process().get_user_token().await;
    let old_mask = signal_state.sigmask;

    if !set_user_ptr.is_null() {
        let set = * current_process().memory_set.lock().await.safe_get_target_ref( set_user_ptr).await?;

        let enum_how = match SigMaskHow::try_from(how) {
            Ok(how) => how,
            Err(_) => return Err(SysErrNo::EINVAL),
        };
        match enum_how {
            SigMaskHow::SIG_BLOCK => {
                signal_state.sigmask.union_with(&set);
            }
            SigMaskHow::SIG_UNBLOCK => {
                // 不能解除 SIGKILL 或 SIGSTOP 的阻塞 
                // 但 SigSet 的操作通常不关心具体信号的特殊性，这是更高层逻辑
                let mut temp_set = set;
                // Linux 不允许 SIGKILL 和 SIGSTOP 被阻塞，所以从 set 中移除它们
                temp_set.remove(Signal::SIGKILL);
                temp_set.remove(Signal::SIGSTOP);
                // 然后从当前掩码中移除这些（解除阻塞）
                // A = A & (~B) => A.intersect_with( !B )
                // SigSet 需要实现 bitwise NOT 或者一个 remove_all_from_set 方法
                // 简单做法：迭代 set 中的每一位，如果在 sigmask 中，则 remove
                // (更正：SIG_UNBLOCK 是移除 set 中的位，所以是 sigmask &= ~set)
                // sigmask = sigmask AND (NOT set)
                // 我们需要一个 SigSet::complement() 或 SigSet::difference_with()
                // 简化：直接迭代要解除阻塞的信号
                for i in 1..NSIG {
                    if let Some(s) = Signal::from_usize(i) {
                        if set.contains(s) {
                            signal_state.sigmask.remove(s);
                        }
                    }
                }
            }
            SigMaskHow::SIG_SETMASK => {
                let mut new_mask = set;
                // 不能阻塞 SIGKILL 或 SIGSTOP
                new_mask.remove(Signal::SIGKILL);
                new_mask.remove(Signal::SIGSTOP);
                signal_state.sigmask = new_mask;
            }
        }
    }

    drop(signal_state); // 先释放锁，再复制到用户空间

    if !oldset_user_ptr.is_null() {
        *current_process().memory_set.lock().await.
        safe_translated_refmut( oldset_user_ptr).await?=old_mask;
        
    }

    Ok(0) // 成功
}
pub async fn sys_kill(target_pid: usize, signum_usize: usize) -> SyscallRet {
    trace!("[sys_kill] target_pid: {}, signum: {}", target_pid, signum_usize);

    // 步骤1：验证信号有效性（复用 tkill 的逻辑）
    let sig = match Signal::from_usize(signum_usize) {
        Some(s) => s,
        None => return Err(SysErrNo::EINVAL), // 无效信号
    };

    // 步骤2：处理信号0（检查进程是否存在）
    if signum_usize == 0 {
        let has_thread = PID2PC.lock().contains_key(&target_pid);
        return if has_thread {
            Ok(0) // 进程存在（至少有一个线程）
        } else {
            Err(SysErrNo::ESRCH) // 进程不存在
        };
    };

    // 步骤3：找到目标 PID 下的所有活跃线程（TID）
     let target_tids: Vec<usize> = PID2PC.lock().get(&target_pid)
    .map_or(Err(SysErrNo::ESRCH), |s|Ok(s))?.tasks.lock().await
    .iter()
    .map(|tcb| tcb.id()) // 提取每个 TCB 的 TID
    .collect() ;

    if target_tids.is_empty() {
        return Err(SysErrNo::ESRCH); // 进程无活跃线程（视为进程不存在）
    }

    // 步骤4：权限检查（TODO：根据实际需求补充，例如检查当前任务是否有权限向目标进程发信号）
    // if !check_permission(current_task.pid, target_pid) {
    //     return Err(SysErrNo::EPERM);
    // }

    // 步骤5：向每个线程发送信号（复用 tkill 的核心逻辑）
    for tid in target_tids {
        // 直接复用 tkill 中“根据 TID 查找任务并发送信号”的逻辑
        let result = sys_tkill(tid, signum_usize).await; 
        
        // 可选：记录失败的线程（根据需求决定是否忽略部分失败）
        if let Err(e) = result {
            warn!("Failed to send signal to tid {} (pid {}): {:?}", tid, target_pid, e);
        }
    }

    Ok(0) // 信号已尝试发送（即使部分线程失败，仍返回成功）
}
// int kill(pid_t pid, int sig); (或 tkill/tgkill)
pub async  fn sys_tkill(target_tid: usize, signum_usize: usize) -> SyscallRet {
    
    trace!("[sys_tkill] target_tid: {}, signum: {}", target_tid, signum_usize);
    let sig = match Signal::from_usize(signum_usize) {
        Some(s) => s,
        None => return Err(SysErrNo::EINVAL), // 无效信号
    };

    if signum_usize == 0 {
        // 发送信号0是检查进程是否存在，不实际发送信号

        if TID2TC.lock().contains_key(&target_tid) {
            return Ok(0); // 存在
        } else {
            return Err(SysErrNo::ESRCH); // 不存在
        }
    }

    let target_task_arc = match TID2TC.lock().get(&target_tid) {
        Some(task_ref) => task_ref.clone(),
        None => return Err(SysErrNo::ESRCH), // No such process/task
    };

    // TODO: 权限检查 (例如，当前任务是否有权限向目标任务发送信号)  @Heliosly.
    // ...

    send_signal_to_task(&target_task_arc, sig).await?;
    Ok(0) // 成功 (信号已加入挂起队列或被处理)
}

// int pause(void);
pub async  fn sys_pause() -> SyscallRet {
    trace!("[sys_pause]");
    let task_arc = current_task();
    // 1. 将当前任务的信号掩码保存起来 (old_mask = task.sigmask)。
    // 2. 将当前任务的信号掩码设置为空 (允许所有信号)。
    //    或者使用一个临时的空掩码。
    // 3. 使任务进入可中断的睡眠状态，直到一个信号被捕获并处理。
    //    这通常通过 `sigsuspend(empty_mask)` 实现。
    //    这里我们简化：
    //    - 检查是否有未阻塞的挂起信号，如果有，handle_pending_signals 会处理，pause 不会阻塞。
    //    - 如果没有，则阻塞，等待任何信号。
    //
    // loop {
    //     handle_pending_signals(&task_arc); // 处理已有的信号
    //     // 如果信号导致任务终止或停止，就不会到这里
    //
    //     // 让任务睡眠，等待被信号唤醒
    //     // 这需要一种机制，比如一个信号专用的条件变量或等待队列
    //     // task_arc.sleep_interruptible_until_signal();
    //     // 如果被唤醒，说明有信号传递，handle_pending_signals 会在返回用户态前再次运行。
    //     // pause 的返回值总是 -EINTR (被信号中断)。
    //     // 如果一个信号处理函数被执行了，pause 就返回了。
    //     // 如果信号的动作是终止，pause 就不会返回。
    //
    //     // 这里需要一个方法让当前任务阻塞，直到有信号处理。
    //     // 一个简单的方法是设置一个flag，然后yield，直到flag被信号处理机制改变。
    //     // 或者，使用一个 Waker，当有信号时被唤醒。
    //     // 这部分逻辑比较复杂，通常与 sigsuspend 紧密相关。
    // }
    // 对于比赛，pause 可以简化为：检查并处理当前挂起信号，
    // 如果没有导致返回的信号（例如，被忽略的或导致终止的），
    // 则让任务进入一个可被任何信号唤醒的阻塞状态。
    // `pause` 总是返回 -EINTR，除非被一个导致进程终止的信号打断。
    Err(SysErrNo::EINTR) // 表示被信号中断
}

