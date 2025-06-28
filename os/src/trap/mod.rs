//! Trap handling functionality

mod context;
mod ucontext;

use crate::syscall::syscall;
use crate::task::{
    current_task, current_task_may_uninit, exit_current, exit_proc, pick_next_task, run_task2, task_count, task_tick, yield_now, CurrentTask, TaskStatus
};
use crate::timer::set_next_trigger;
use crate::utils::error::{GeneralRet, SysErrNo};
pub use context::user_return;
pub use context::TrapStatus;
use loongArch64::register::estat;
use core::future::poll_fn;
use core::panic;
use core::task::Poll;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_SET};
pub use ucontext::{MContext, UContext};
use crate::arch::CurrentTrap;
use crate::arch::TrapArch;

pub fn init() {
    CurrentTrap::init_trap();
}

pub fn set_sum() {
    CurrentTrap::set_sum();
}

pub fn enable_irqs() {
    CurrentTrap::enable_irqs();
}

pub fn disable_irqs() {
    CurrentTrap::disable_irqs();
}

#[inline]
pub fn enable_kernel_irqs() {
    CurrentTrap::enable_kernel_irqs();
}

#[inline]
pub fn disable_kernel_irqs() {
    CurrentTrap::disable_kernel_irqs();
}

#[inline]
pub fn wait_for_irqs() {
    CurrentTrap::wait_for_irqs();
}

#[no_mangle]
pub fn trap_from_kernel() {
    let scause = CurrentTrap::read_scause();
    let stval = CurrentTrap::read_stval();
    let sepc = CurrentTrap::read_sepc();
   
   
        panic!(
            "stval = {:#x}, sepc = {:#x},scause:{:#?}, a trap from kernel",
            stval,
            sepc,
            estat::read().cause()
        );
    
}

pub use context::TrapContext;

#[no_mangle]

pub fn trampoline(_tc: *mut TrapContext, has_trap: bool, from_user: bool) {
    loop {
        if !from_user && has_trap {
            trap_from_kernel();
            return;
        } else {
            crate::task::sleeplist::process_timed_events();

            if let Some(curr) = CurrentTask::try_get().or_else(|| {
                if let Some(task) = pick_next_task() {
                    unsafe {
                        CurrentTask::init_current(task);
                    }
                    let res = CurrentTask::get();
                    Some(res)
                } else {
                    None
                }
            }) {
                //  debug!("run_task pid:{},Arc count:{}",curr.pid.0,

                //                    Arc::strong_count(&curr)
                //                        );

                // trace!("run task tid = {}", curr.id());
                run_task2(CurrentTask::from(curr));
            } else {
                enable_irqs();
                wait_for_irqs();
            }
        }
    }
}

fn log_page_fault_error(stval: usize, sepc: usize) {
    println!(
        "[kernel] trap_handler: PageFault in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
        stval,
        sepc
    );
}

pub async fn user_task_top() -> i32 {
    loop {
        debug!("into user_task_top");
        let curr = current_task();
  
        let mut syscall_ret = None;
        let tf = curr.get_trap_cx().unwrap();
        
        // println!("tf p:{:#p}debug:{:#?}",tf,tf);
        if tf.trap_status == TrapStatus::Blocked {
            let scause = CurrentTrap::read_scause();
            let stval = CurrentTrap::read_stval();
            let sepc = CurrentTrap::read_sepc();
            let satp = CurrentTrap::read_satp();
              #[cfg(target_arch = "loongarch64")]
                trace!(
                    "stval = {:#x}, sepc = {:#x},scause:{:#?},into top",
                    stval,
                    sepc,
                    estat::read().cause()
                );
            trace!(
                "Trap: addr={:#x}, sepc={:#x}, satp={:#x},scause:{:#x}",
                stval,
                sepc,
                satp,
                scause.code()
            );
            
            if CurrentTrap::is_syscall(&scause) {
                enable_irqs();
                let syscall_id = tf.regs.a7;

    // println!("debug:{:#?}",tf);
                debug!("[user_task_top]sys_call start syscall id = {} tid = {},pid={},sepc:{:#x},a0:{}",
                    syscall_id, curr.id(), curr.get_process().unwrap().get_pid(), sepc, tf.regs.a0);

                let args = [
                    tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3, tf.regs.a4, tf.regs.a5,
                ];

                tf.sepc += CurrentTrap::syscall_instruction_len();

                let result = syscall(syscall_id, args).await;

                curr.update_stime();
                let result = match result {
                    Ok(res) => res,
                    Err(err) => {
                        if err == SysErrNo::EAGAIN {
                            tf.sepc -= CurrentTrap::syscall_instruction_len();
                            debug!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                            yield_now().await;
                            tf.regs.a0
                        } else if err == SysErrNo::ECHILD {
                            debug!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                            -(err as isize) as usize
                        } else if err == SysErrNo::EINVAL {
                            println!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                            -(err as isize) as usize
                        } else {
                            warn!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                            -(err as isize) as usize
                        }
                        // debug!("[Syscall]Err:{}", err.str());
                    }
                };
               
                tf.set_origin_a0(tf.regs.a0);
                tf.set_arg0(result);
                syscall_ret = Some(result);

                // trace!("sys_call end1"); 
                // 判断任务是否退出
                if curr.is_exited() {
                    // 任务结束，需要切换至其他任务，关中断
                    disable_irqs();
                    return curr.get_exit_code() as i32;
                }

                disable_irqs();
            } else if CurrentTrap::is_page_fault(&scause) || CurrentTrap::is_illegal_instruction(&scause) {
                //懒分配
                // println!(
                //     "[kernel] trap_handler:  PageFault in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
                //     stval,
                //     sepc
                // );
                let fault_addr = if CurrentTrap::is_illegal_instruction(&scause) {
                    // 如果是非法指令异常，fault_addr 是 sepc
                    sepc
                } else {
                    stval
                };
                
                if curr
                    .get_process()
                    .unwrap()
                    .memory_set
                    .lock()
                    .await
                    .handle_page_fault(fault_addr)
                    .await
                    .is_err()
                {
                    exit_current(-2).await;
                    log_page_fault_error(stval, sepc);
                }
            } else if CurrentTrap::is_store_fault(&scause) 
                || CurrentTrap::is_instruction_fault(&scause)
                || CurrentTrap::is_load_fault(&scause) {
                println!(
                    "[kernel] trap_handler: Fault in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it....",
                    stval,
                    tf.sepc,
                );
                // page fault exit code
                exit_current(-2).await;
            } else if CurrentTrap::is_breakpoint(&scause) {
                println!("[kernel] Breakpoint exception in application (sepc={:#x}). Probably from abort(). Terminating process.", tf.sepc);
                exit_current(-4).await;
            } else if CurrentTrap::is_timer_interrupt(&scause) {
                set_next_trigger();
                tf.trap_status = TrapStatus::Done;
                on_timer_tick();
                if let Some(curr) = current_task_may_uninit() {
                    // if task is already exited or blocking,
                    // no need preempt, they are rescheduling
                    if curr.need_resched()
                        && curr.can_preempt()
                        && !curr.is_exited()
                        && !curr.is_blocking()
                    {
                        trace!(
                            "[user_task_top]current {} is to be preempted in user mode, allow {},a0:{:#x}",
                            curr.id(),
                            curr.can_preempt(),
                            tf.regs.a0
                        );
                        curr.set_need_resched(false);
                        tf.trap_status = TrapStatus::Blocked;
                        yield_now().await;
                    }
                }
            } else {

                #[cfg(target_arch = "riscv64")]
                panic!(
                    "Unsupported trap, stval = {:#x}!",
                    stval
                );
                #[cfg(target_arch = "loongarch64")]
                panic!(
                    "stval = {:#x}, sepc = {:#x},scause:{:#?}, Unsupport trap",
                    stval,
                    sepc,
                    estat::read().cause()
                );
            }

            {
                crate::signal::handle_pending_signals(syscall_ret).await;
                crate::task::sleeplist::process_timed_events();
            }

            tf.trap_status = TrapStatus::Done;
            if curr.is_zombie() || curr.get_process().is_none() {
                disable_irqs();
                return curr.get_exit_code() as i32;
            } else {
                if let Some(pcb) = curr.get_process() {
                    if *pcb.state.lock().await == TaskStatus::Zombie {
                        disable_irqs();
                        return pcb.exit_code();
                    }
                }
            }
            disable_irqs();
        }

        poll_fn(|_cx| {
            if tf.trap_status == TrapStatus::Done {
                Poll::Pending
            } else {
                Poll::Ready(0)
            }
        })
        .await;
    }
}

pub fn on_timer_tick() {
    if let Some(curr) = current_task_may_uninit() {
        if task_tick(curr.as_task_ref()) {
            curr.set_need_resched(true);
        }
    }
}