/// # 信号处理模块
///
/// 该模块实现了信号处理机制，包括信号的发送、接收、阻塞、忽略和处理。
/// 信号是一种进程间通信的方式，用于通知进程发生了某些事件。

/// ## 任务和进程信号状态
///
/// `TaskSignalState` 结构体包含了任务的信号状态，如挂起的信号、阻塞的信号和备选信号栈。
/// `ProcessSignalSharedState` 结构体包含了进程共享的信号处理动作和挂起的信号。
///
/// ## 信号发送
///
/// `send_signal` 函数用于向目标进程或线程发送信号。
/// `send_signal_to_task` 函数用于向指定任务发送信号。
///
/// ## 信号处理
///
/// `handle_pending_signals` 函数用于处理挂起的信号，包括线程独有的和进程共享的信号。
/// `load_trap_for_signal` 函数用于恢复被信号处理函数打断前的 TrapFrame，准备返回用户态。
/// `perform_default_action_for_process` 函数用于执行信号的默认动作。
///

/// ## 错误处理
///
/// `SignalError` 枚举定义了信号处理过程中可能发生的错误，如无效信号、没有找到进程或线程等。
// pub mod sigact;

//
mod sigact;
mod signal;
use alloc::sync::Arc;
pub use sigact::*;
pub use signal::*;
use crate::config::{ SS_DISABLE, USER_SIGNAL_PROTECT};
use crate::mm::put_data;
use crate::task::{current_task, exit_proc};
use crate::task::{ ProcessRef, Task, TaskRef, PID2PC}; // 确保 Task 有 id()
use crate::trap::{TrapContext, UContext};
use crate::utils::error::SysErrNo; // 假设 current_task() 返回 Arc<Task>

// 通常信号编号从 1 开始。0 不是有效信号。
pub const NSIG: usize = 64; // 支持的信号数量 (Linux x86_64 通常是64)
extern "C" {
    fn start_signal_trampoline();
}

/// 向目标发送信号
/// target_pid: 目标进程的 PID
/// target_tid: 可选的目标线程的 TID (如果为 None，则发给整个进程)
/// sig: 要发送的信号
pub async fn send_signal(
    target_pid: usize,
    target_tid: Option<usize>,
    sig: Signal,
) -> Result<(), SignalError> {
    if sig as usize == 0 || sig as usize >= NSIG {
        return Err(SignalError::InvalidSignal);
    }

    // 1. 找到目标进程的 PCB
    let pcb_arc = PID2PC
        .lock() 
        .get(&target_pid)
        .cloned()
        .ok_or(SignalError::NoSuchProcess)?;

    // TODO: 权限检查 (当前进程是否有权限向目标进程/线程发送信号) @Heliosly.
    // ...

    if let Some(tid) = target_tid {
        // --- 发送给特定线程 (tkill / pthread_kill 语义) ---
        let task_arc = pcb_arc
            .find_task_by_tid(tid)
            .await
            .ok_or(SignalError::NoSuchThread)?; // PCB 需要此方法
        let mut task_signal_state = task_arc.signal_state.lock().await;
        let process_signal_state = pcb_arc.signal_shared_state.lock(); // 需要读取 sigactions

        let action = &process_signal_state.sigactions[sig as usize];
        if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
            return Ok(()); // 忽略
        }
        task_signal_state.sigpending.add(sig);
        drop(task_signal_state);
        drop(process_signal_state);

        // 尝试唤醒目标线程 (如果它可被中断)
        // target_task_arc.try_interrupt_if_blocked();
        let task_ptr: *const Task = Arc::as_ptr(&task_arc);
        unsafe { crate::task::waker::wakeup_task(task_ptr) }; // 假设可以安全调用
    } else {
        // --- 发送给整个进程 (kill 语义) ---
        let mut process_signal_state = pcb_arc.signal_shared_state.lock();
        let action = &process_signal_state.sigactions[sig as usize];

        if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
            return Ok(()); // 整个进程忽略此信号
        }
        process_signal_state.shared_sigpending.add(sig);
        drop(process_signal_state);

        // 选择一个合适的线程来传递这个进程信号 (或者唤醒所有可中断的线程)
        // Linux 的策略比较复杂：
        // - 如果有线程专门等待这个信号 (sigwaitinfo)，唤醒它。
        // - 否则，选择一个没有阻塞该信号的线程。
        // - 如果所有线程都阻塞了该信号，信号保持在进程挂起队列。
        // - 优先选择主线程或当前线程（如果发送给自己）。
        //
        // 简化版：唤醒进程中的一个（或所有）可被中断的线程。
        // 这可能需要迭代 pcb_arc.tasks。
        let tasks_guard = pcb_arc.tasks.lock();
        for task_ref in tasks_guard.iter() {
            // if task_ref.is_interruptible() && !task_ref.signal_thread_state.lock().sigmask.contains(sig) {
            //     task_ref.try_interrupt_if_blocked();
            //     break; // 通常只唤醒一个来处理进程信号
            // }
            // 简化：尝试唤醒第一个（或主线程）
            // 这里的唤醒是指让调度器有机会运行它，以便它能检查信号
            let task_ptr: *const Task = Arc::as_ptr(task_ref);
            unsafe { crate::task::waker::wakeup_task(task_ptr) };
            break; // 仅唤醒一个
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum SignalError {
    InvalidSignal,
    NoSuchProcess,
    NoSuchThread,
    PermissionDenied,
}

impl From<SignalError> for SysErrNo {
    fn from(err: SignalError) -> Self {
        match err {
            SignalError::InvalidSignal => SysErrNo::EINVAL,
            SignalError::NoSuchProcess => SysErrNo::ESRCH,
            SignalError::NoSuchThread => SysErrNo::ESRCH,
            SignalError::PermissionDenied => SysErrNo::EPERM,
        }
    }
}

/// 向指定任务发送一个信号（类似 `tkill`/`pthread_kill`）
/// - `task_arc`：目标任务引用
/// - `sig`：要发送的信号
/// 返回 Err 表示信号号无效或没有权限等
pub async fn send_signal_to_task(task_arc: &Arc<Task>, sig: Signal) -> Result<(), SignalError> {
    // 1. 检查信号号有效性
    let signum = sig as usize;
    if signum == 0 || signum >= NSIG {
        return Err(SignalError::InvalidSignal);
    }

    // 2. 获取进程共享的 sigactions（注册的处理方式）
    //    假设 Task 有方法 .get_pcb() 拿到它所属的进程控制块
    let pcb_arc = task_arc.get_process();
    let proc_sig_shared = pcb_arc.signal_shared_state.lock().await;
    let action = proc_sig_shared.sigactions[signum];
    // SIGKILL 和 SIGSTOP 永远不能被忽略
    if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
        return Ok(());
    }
    drop(proc_sig_shared);

    // 3. 将信号加入该线程的挂起集合
    {
        let mut task_sig_state = task_arc.signal_state.lock().await;
        task_sig_state.sigpending.add(sig);
    }

    // 4. 唤醒该任务，让调度器有机会运行它，
    //    以便它在回用户态前调用 handle_pending_signals
    let task_ptr: *const Task = Arc::as_ptr(task_arc);
    crate::task::waker::wakeup_task(task_ptr);

    Ok(())
}
pub async  fn handle_pending_signals() {
    let task_arc = current_task();
    let pcb_arc=&task_arc.get_process();
    // 1. 获取线程和进程的信号状态锁
    let mut task_state = task_arc.signal_state.lock().await;
    let mut process_state = pcb_arc.signal_shared_state.lock().await;
    let token = unsafe { *task_arc.page_table_token.get() };
    loop {
        // 可能有多个信号需要处理
        let mut signal_to_deliver: Option<Signal> = None;
        let mut delivered_from_thread_pending = false;

        // a. 优先检查并处理线程独有的、未被阻塞的挂起信号
        for signum_idx in 1..NSIG {
            if let Some(sig) = Signal::from_usize(signum_idx) {
                if task_state.sigpending.contains(sig) && !task_state.sigmask.contains(sig) {
                    signal_to_deliver = Some(sig);
                    delivered_from_thread_pending = true;
                    break;
                }
            }
        }

        // b. 如果没有线程独有的，则检查进程共享的、未被此线程阻塞的挂起信号
        if signal_to_deliver.is_none() {
            for signum_idx in 1..NSIG {
                if let Some(sig) = Signal::from_usize(signum_idx) {
                    if process_state.shared_sigpending.contains(sig)
                        && !task_state.sigmask.contains(sig)
                    {
                        signal_to_deliver = Some(sig);
                        delivered_from_thread_pending = false; // 来自进程队列
                        break;
                    }
                }
            }
        }

        if let Some(sig) = signal_to_deliver {
            debug!("[handle_pending_signals],signal to deliver sig:{:#?}",sig);
            let action = process_state.sigactions[sig as usize].clone(); // 动作是进程共享的

            // 从相应的挂起队列中移除
            if delivered_from_thread_pending {
                task_state.sigpending.remove(sig);
            } else {
                process_state.shared_sigpending.remove(sig);
                // 重要：如果这个信号是发给进程的，理论上只有一个线程会处理它。
                // 其他线程不应该再看到这个进程挂起信号（除非是广播信号或特殊情况）。
                // 我们的模型是，一旦一个线程选中了一个进程信号来传递，就从共享队列移除。
            }

            // 特殊处理 SIGKILL 和 SIGSTOP (它们不能被捕获或忽略，动作是固定的)
            if sig == Signal::SIGKILL {
                exit_proc((128+sig as usize) as i32).await;
                log::info!("Process {} (task {}) received SIGKILL, terminating all tasks.", pcb_arc.pid.0, task_arc.id());
                return; // 进程终止，无需继续
            }
            if sig == Signal::SIGSTOP {
                log::info!("Process {} (task {}) received SIGSTOP, stopping all tasks.", pcb_arc.pid.0, task_arc.id());
                unimplemented!();
                continue;
            }
            // 同样，SIGCONT 需要唤醒进程中的所有线程
            if sig == Signal::SIGCONT {
                unimplemented!();
                // log::info!("Process {} (task {}) received SIGCONT, continuing all tasks.", pcb_arc.pid.0, task_arc.id());
                // pcb_arc.continue_all_tasks(); // 假设有此方法
                // SIGCONT 的默认动作是 Ignore (如果之前是Stop) 或 Continue。
                // 如果有用户处理器，则执行用户处理器。
                // 我们这里先按默认处理，如果用户有处理器，下面会走到。
            }

            // 计算在信号处理函数执行期间需要阻塞的掩码
            let mut new_mask_during_handler = task_state.sigmask; // 基于线程当前掩码
            if !action.flags.contains(SigActionFlags::SA_NODEFER) {
                new_mask_during_handler.add(sig);
            }
            new_mask_during_handler.union_with(&action.mask); // 加上 sa_mask

            // 释放锁，因为执行动作或准备用户态帧可能需要时间，或发生上下文切换
            let original_thread_mask = task_state.sigmask; // 保存原始掩码以备恢复
            task_state.sigmask = new_mask_during_handler; // 临时设置新掩码
            drop(task_state);
            drop(process_state); // 释放两个锁

            // -- 执行动作 --
            match action.handler {
                SIG_DFL => {
                    unsafe { perform_default_action_for_process(pcb_arc, &task_arc.0, sig) };
                    // 默认动作可能影响整个进程
                }
                SIG_IGN => { /* 忽略 */ }
                user_handler_addr => {
                    // log::info!("Task {} (in process {}) delivering signal {:?} to handler 0x{:x}",
                    //          task_arc.id(), pcb_arc.pid.0, sig, user_handler_addr);
                    let sig_num: usize = sig.into();
                    // 此时需要调用信号处理函数，注意调用的方式是：
                    // 通过修改trap上下文的pc指针，使得trap返回之后，直接到达信号处理函数
                    // 因此需要处理一系列的trap上下文，使得正确传参与返回。
                    // 具体来说需要考虑两个方面：
                    // 1. 传参
                    // 2. 返回值ra地址的设定，与是否设置了SA_RESTORER有关

                    // 读取当前的trap上下文
                    // let mut trap_frame = read_trapframe_from_kstack(current_task.get_kernel_stack_top().unwrap());
                    let mut task_state = task_arc.signal_state.lock().await;

                    let trap_frame = task_arc.get_trap_cx().unwrap();

                    // // 新的trap上下文的sp指针位置，由于SIGINFO会存放内容，所以需要开个保护区域
                    let mut sp = if action.flags.contains(SigActionFlags::SA_ONSTACK)
                        && task_state.alternate_stack.flags != SS_DISABLE
                    {
                        debug!("Use alternate stack");
                        // Use alternate stack
                        (task_state.alternate_stack.sp + task_state.alternate_stack.size - 1) & !0xf
                    } else {
                        trap_frame.get_sp() - USER_SIGNAL_PROTECT
                    };

                    info!("use stack: {:#x}", sp);
                    let restorer = if let Some(addr) = action.get_restorer() {
                        addr
                    } else {
                        start_signal_trampoline as usize
                    };

                    info!(
                        "restorer :{:#x}, handler: {:#x}",
                        restorer, user_handler_addr
                    );
                    #[cfg(not(target_arch = "x86_64"))]
                    trap_frame.set_ra(restorer);

                    let old_pc = trap_frame.get_pc();

                    trap_frame.set_pc(user_handler_addr);
                    // 传参
                    trap_frame.set_arg0(sig.into());
                    // 若带有SIG_INFO参数，则函数原型为fn(sig: SignalNo, info: &SigInfo, ucontext: &mut UContext)
                    if action.flags.contains(SigActionFlags::SA_SIGINFO) {
                        // current_task.set_siginfo(true);
                        task_state.sig_info = true;
                        let sp_base: usize = (((sp - core::mem::size_of::<SigInfo>()) & !0xf)
                            - core::mem::size_of::<UContext>())
                            & !0xf;

                        // TODO: 统一为访问用户空间的操作封装函数
                        // process
                        //     .manual_alloc_range_for_lazy(sp_base.into(), sp.into())
                        //     .await
                        //     .expect("Failed to alloc memory for signal user stack");

                        // 注意16字节对齐
                        sp = (sp - core::mem::size_of::<SigInfo>()) & !0xf;
                        let info = SigInfo {
                            si_signo: sig_num as u32,
                            ..Default::default()
                        };
                        unsafe {
                            *(sp as *mut SigInfo) = info;
                        }
                        trap_frame.set_arg1(sp);

                        // 接下来存储ucontext
                        sp = (sp - core::mem::size_of::<UContext>()) & !0xf;

                        let ucontext = UContext::init(old_pc, original_thread_mask);
                        put_data(token, sp as *mut UContext, ucontext).unwrap();

                        trap_frame.set_arg2(sp);
                    }

                    trap_frame.set_sp(sp);
                    if action.flags.contains(SigActionFlags::SA_RESETHAND) {
                        let mut temp_proc_state = pcb_arc.signal_shared_state.lock().await;
                        temp_proc_state.sigactions[sig as usize].handler = SIG_DFL;
                    }
                    return; // 信号已交付给用户处理程序，本次内核处理结束
                }
            }
            // 如果执行到这里（例如 SIG_IGN 或某些不终止的 SIG_DFL），重新获取锁并继续循环
            task_state = task_arc.signal_state.lock().await;
            process_state = pcb_arc.signal_shared_state.lock().await;
            task_state.sigmask = original_thread_mask; // 恢复掩码（如果没进用户处理函数）
        } else {
            // 没有需要处理的信号了
            // drop(thread_state); // 在循环外统一 drop
            // drop(process_state);
            break;
        }
    }
    // 确保锁在这里被释放
}

/// 恢复被信号处理函数打断前的 TrapFrame，准备返回用户态。
/// 如果存在 saved_trap，则返回 true（表示已装载），否则返回 false。
#[no_mangle]
pub async fn load_trap_for_signal() -> bool {
    // 1. 找到当前任务
    let task = current_task();
    // 2. 拿到它的信号状态
    let mut sig_state = task.signal_state.lock().await;

    // 3. 如果之前有保存的 TrapFrame，就拿出来
    if let Some(saved_frame) = sig_state.last_context.take() {
        unsafe {
            // 4. 拿到实际用于中断/异常的内核栈上的 TrapFrame 指针
            let now_trap_frame: &mut TrapContext = task.get_trap_cx().unwrap();

            // 5. 拷贝回去，恢复原先全部寄存器状态
            *now_trap_frame = saved_frame;

            // 6. 如果当时用的是 SA_SIGINFO（sig_info = true），
            //    需要从用户栈上的 UContext 里再拿一次 PC
            if sig_state.sig_info {
                // 用户态信号上下文结构在用户栈顶
                let sp = now_trap_frame.regs.sp;
                let user_ctx = &*(sp as *const UContext);
                let pc = user_ctx.get_pc();
                now_trap_frame.set_pc(pc);
            }
        }
        true
    } else {
        // 没有未处理的信号上下文
        false
    }
}

pub fn perform_default_action_for_process(
    pcb_arc: &ProcessRef,
    _current_task_arc: &TaskRef,
    sig: Signal,
) {
    // 默认动作现在可能需要作用于整个进程
    match sig.default_action() {
        SignalDefaultAction::Terminate | SignalDefaultAction::CoreDump => {
            // log::info!("Process {} terminating due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.terminate_all_tasks_and_self();
        }
        SignalDefaultAction::Ignore => {}
        SignalDefaultAction::Stop => {
            // log::info!("Process {} stopping due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.stop_all_tasks();
        }
        SignalDefaultAction::Continue => {
            // log::info!("Process {} continuing due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.continue_all_tasks();
        }
        SignalDefaultAction::ForceTerminateOrStop => {
            unreachable!("SIGKILL/SIGSTOP default actions should be handled earlier in handle_pending_signals");
        }
    }
}



