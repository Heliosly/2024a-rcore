// pub mod sigact;
pub mod signal;

// use core::mem::size_of;

// use alloc::sync::Arc;
// use log::debug;
// use riscv::register::scause::{self, Exception, Trap};
// pub use sigact::*;
// pub use signal::*;

// use crate::{
//     config::mm::USER_STACK_SIZE,
//     mm::{get_data, put_data},
//     task::{current_task, exit_current_and_run_next, TaskControlBlock, THREAD_GROUP, TID_TO_TASK},
//     trap::{MachineContext, UserContext},
//     utils::{SysErrNo, SyscallRet},
// };

// pub const SIG_MAX_NUM: usize = 33;
// pub const SIG_ERR: usize = usize::MAX;
// pub const SIG_DFL: usize = 0;
// pub const SIG_IGN: usize = 1;

// extern "C" {
//     pub fn sigreturn_trampoline();
// }

// pub fn check_if_any_sig_for_current_task() -> Option<usize> {
//     let task = current_task().unwrap();
//     let task_inner = task.inner_lock();

//     task_inner
//         .sig_pending
//         .difference(task_inner.sig_mask)
//         .peek_front()
// }

// pub fn handle_signal(signo: usize) {
//     let task = current_task().unwrap();
//     let mut task_inner = task.inner_lock();
//     let signal = SigSet::from_sig(signo);
//     debug!("[handle_signal] signo={},handle signal {:?}", signo, signal);
//     let sig_action = task_inner.sig_table.action(signo);
//     task_inner.sig_pending.remove(signal);
//     drop(task_inner);
//     drop(task);
//     if sig_action.customed {
//         setup_frame(signo, sig_action);
//     } else {
//         debug!("sa_handler:{:#x}", sig_action.act.sa_handler as usize);
//         // 就在S模式运行,转换成fn(i32)
//         if sig_action.act.sa_handler != 1 {
//             if sig_action.act.sa_handler == exit_current_and_run_next as usize {
//                 exit_current_and_run_next((signo + 128) as i32);
//             }
//         }
//     }
// }
// /// 在用户态栈空间构建一个 Frame
// /// 构建这个帧的目的就是为了执行完信号处理程序后返回到内核态，
// /// 并恢复原来内核栈的内容
// pub fn setup_frame(signo: usize, sig_action: KSigAction) {
//     debug!("customed sa_handler={:#x}", sig_action.act.sa_handler);

//     let task = current_task().unwrap();
//     let mut task_inner = task.inner_lock();
//     let token = task_inner.user_token();

//     let trap_cx = task_inner.trap_cx();
//     let mut user_sp = trap_cx.gp.x[2];

//     // if this syscall wants to restart
//     if scause::read().cause() == Trap::Exception(Exception::UserEnvCall)
//         && trap_cx.gp.x[10] == SysErrNo::ERESTART as usize
//     {
//         // and if `SA_RESTART` is set
//         if sig_action.act.sa_flags.contains(SigActionFlags::SA_RESTART) {
//             debug!("[do_signal] syscall will restart after sigreturn");
//             // back to `ecall`
//             trap_cx.sepc -= 4;
//             // restore syscall parameter `a0`
//             trap_cx.gp.x[10] = trap_cx.origin_a0;
//         } else {
//             debug!("[do_signal] syscall was interrupted");
//             // will return EINTR after sigreturn
//             trap_cx.gp.x[10] = SysErrNo::EINTR as usize;
//         }
//     }

//     if !sig_action.act.sa_flags.contains(SigActionFlags::SA_SIGINFO) {
//         // 处理函数 (*sa_handler)(int);
//         // 保存 Trap 上下文
//         user_sp = user_sp - size_of::<MachineContext>();
//         put_data(token, user_sp as *mut MachineContext, trap_cx.as_mctx());

//         // signal mask
//         user_sp = user_sp - size_of::<SigSet>();
//         put_data(token, user_sp as *mut SigSet, task_inner.sig_mask);

//         // 不是 sigInfo
//         user_sp = user_sp - size_of::<usize>();
//         put_data(token, user_sp as *mut usize, 0);
//     } else {
//         // (*sa_sigaction)(int, siginfo_t *, void *) 第三个参数指向UserContext
//         let uctx_addr = user_sp - size_of::<UserContext>();
//         let siginfo_addr = uctx_addr - size_of::<SigInfo>();
//         let sig_sp = siginfo_addr;
//         let sig_size = sig_sp - (task_inner.user_stack_top - USER_STACK_SIZE);
//         // debug!("sig_size={:#x}", sig_size);
//         put_data(
//             token,
//             uctx_addr as *mut UserContext,
//             UserContext {
//                 flags: 0,
//                 link: 0,
//                 stack: SignalStack::new(sig_sp, sig_size),
//                 sigmask: task_inner.sig_mask,
//                 __pad: [0u8; 128],
//                 mcontext: trap_cx.as_mctx(),
//             },
//         );
//         // a2
//         trap_cx.gp.x[12] = uctx_addr;
//         put_data(
//             token,
//             siginfo_addr as *mut SigInfo,
//             SigInfo::new(signo, 0, 0),
//         );
//         // a1
//         trap_cx.gp.x[11] = siginfo_addr;

//         user_sp = sig_sp;
//         // 是 sigInfo
//         user_sp = user_sp - size_of::<usize>();
//         put_data(token, user_sp as *mut usize, usize::MAX);
//     }

//     // checkout(Magic Num)
//     user_sp -= size_of::<usize>();
//     put_data(token, user_sp as *mut usize, 0xdeadbeef);
//     // a0
//     trap_cx.gp.x[10] = signo;
//     // sp
//     trap_cx.set_sp(user_sp);
//     // 修改Trap
//     trap_cx.sepc = sig_action.act.sa_handler;
//     // ra
//     trap_cx.gp.x[1] = if sig_action
//         .act
//         .sa_flags
//         .contains(SigActionFlags::SA_RESTORER)
//     {
//         sig_action.act.sa_restore
//     } else {
//         sigreturn_trampoline as usize
//     };
//     task_inner.sig_mask |= sig_action.act.sa_mask | SigSet::from_sig(signo);
// }
// /// 恢复栈帧
// pub fn restore_frame() -> SyscallRet {
//     let task = current_task().unwrap();
//     let mut task_inner = task.inner_lock();
//     let token = task_inner.user_token();

//     let trap_cx = task_inner.trap_cx();
//     let mut user_sp = trap_cx.gp.x[2];

//     let checkout = get_data(token, user_sp as *const usize);
//     assert!(checkout == 0xdeadbeef, "restore frame checkout error!");
//     user_sp += size_of::<usize>();

//     // sigInfo标志位
//     let sa_siginfo = get_data(token, user_sp as *const usize) == usize::MAX;
//     user_sp += size_of::<usize>();

//     if !sa_siginfo {
//         // signal mask
//         task_inner.sig_mask = get_data(token, user_sp as *const SigSet);
//         user_sp += size_of::<SigSet>();
//         // Trap cx
//         let mctx = get_data(token, user_sp as *const MachineContext);
//         trap_cx.copy_from_mctx(mctx);
//     } else {
//         user_sp += size_of::<SigInfo>();
//         task_inner.sig_mask = get_data(
//             token,
//             (user_sp + 2 * size_of::<usize>() + size_of::<SignalStack>()) as *const SigSet,
//         );
//         let mctx = get_data(
//             token,
//             (user_sp
//                 + 2 * size_of::<usize>()
//                 + size_of::<SignalStack>()
//                 + size_of::<SigSet>()
//                 + 128) as *mut MachineContext,
//         );
//         trap_cx.copy_from_mctx(mctx);
//     }
//     debug!("[restore_frame!] sepc= {:#x}", trap_cx.sepc);
//     Ok(trap_cx.gp.x[10])
// }

// pub fn add_signal(task: Arc<TaskControlBlock>, signal: SigSet) {
//     let mut task_inner = task.inner_lock();
//     task_inner.sig_pending |= signal;
//     // if task_inner.task_status == TaskStatus::Stopped {
//     //     task_inner.task_status = TaskStatus::Ready
//     // }
//     // drop(task_inner);
//     // wakeup_stopped_task(task);
// }

// pub fn send_signal_to_thread_group(pid: usize, sig: SigSet) {
//     let thread_group = THREAD_GROUP.lock();
//     if let Some(tasks) = thread_group.get(&pid) {
//         for task in tasks.iter() {
//             add_signal(task.clone(), sig);
//         }
//     }
// }

// pub fn send_signal_to_thread(tid: usize, sig: SigSet) {
//     let tid2task = TID_TO_TASK.lock();
//     if let Some(task) = tid2task.get(&tid) {
//         add_signal(Arc::clone(task), sig);
//     }
// }

// pub fn send_signal_to_thread_of_proc(pid: usize, tid: usize, sig: SigSet) {
//     let tid2task = TID_TO_TASK.lock();
//     if let Some(task) = tid2task.get(&tid) {
//         if task.pid() == pid {
//             add_signal(Arc::clone(task), sig);
//         }
//     }
// }

// // 目前的进程组只是一个进程的所有子进程的集合
// pub fn send_signal_to_process_group(_pid: usize, _sig: SigSet) {
//     todo!()
// }

// /// 向除自身以及 `init` 进程之外的所有进程发送信号
// pub fn send_access_signal(tid: usize, sig: SigSet) {
//     TID_TO_TASK
//         .lock()
//         .iter()
//         .filter(|(k, _)| **k != tid && **k != 0)
//         .for_each(|(_, task)| add_signal(task.clone(), sig));
// }
