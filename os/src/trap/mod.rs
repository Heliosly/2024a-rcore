//! Trap handling functionality

mod context;
mod ucontext;
use crate::arch::{scause, sepc, stval, CurrentTrap, Exception, Interrupt, Scause, TrapArch};
use crate::syscall::syscall;
use crate::task::{
     current_task, current_task_may_uninit, exit_current, exit_proc, pick_next_task, run_task2,  task_tick, yield_now, CurrentTask, TaskStatus
};
use crate::timer::set_next_trigger;
use crate::utils::error::SysErrNo;
pub use context::user_return;
use crate::arch::Trap;
pub use context::TrapStatus;
use core::future::poll_fn;
use core::panic;
use core::task::Poll;

pub use ucontext::{MContext, UContext};

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

#[cfg(target_arch="riscv64")]
#[no_mangle]
pub fn trap_from_kernel() {
    // backtrace();
    let stval = stval::read();
    let sepc = sepc::read();
    // let stval_vpn = VirtPageNum::from(stval);
    // let sepc_vpn = VirtPageNum::from(sepc);
    let scause = scause::read();
    match scause.cause() {
        Trap::Interrupt(Interrupt::SupervisorTimer) => {}
        _ => {
            panic!(
                "stval = {:#x}, sepc = {:#x},
        a trap {:?} from kernel",
                stval,
                sepc,
                scause::read().cause()
            );
        }
    }
}
#[cfg(target_arch="loongarch64")]
#[no_mangle]
pub fn trap_from_kernel() {
    // backtrace();
    let stval = stval::read();
    let sepc = sepc::read();
    // let stval_vpn = VirtPageNum::from(stval);
    // let sepc_vpn = VirtPageNum::from(sepc);
    let scause = scause::read();
    match scause.cause() {
        _ => {
            panic!(
                "stval = {:#x}, sepc = {:#x},
        a trap {:?} from kernel",
                stval,
                sepc,
                scause::read().cause()
            );
        }
    }
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

            // debug!("into trampoline from taskcount:{},task",task_count());
            // 用户态发生了 Trap 或者需要调度
            if let Some(curr) = CurrentTask::try_get().or_else(|| {
                if let Some(task) = pick_next_task() {
                    unsafe {
                        CurrentTask::init_current(task);
                    }
                    let res = CurrentTask::get();
                    // trace!("take a task tid = {}",res.id());

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
                error!("no tasks available in run_tasks");

                wait_for_irqs();
            }
        }
    }
}
fn log_page_fault_error(scause: Scause, stval: usize, sepc: usize) {
    println!(
        "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
        scause.cause(),
        stval,
        sepc
    );
}

///a future to handle user trap
/// IMPO

#[cfg(target_arch="loongarch64")]
pub async fn user_task_top() -> i32 {
    loop {
        debug!("into user_task_top");
        let curr = current_task();

        let mut syscall_ret = None;
        let tf = curr.get_trap_cx().unwrap();
        // debug!("trap_status:{:?}",tf.trap_status);
        if tf.trap_status == TrapStatus::Blocked {
            let scause = scause::read();
            let stval = stval::read();
            let sepc = sepc::read();
      
            // if curr.get_pid()==3{
            //     trace!(
            //         "Trap:{:#?}",tf
            //     )
            // }
            match scause.cause() {
                Trap::Exception(Exception::Syscall) => {
                    enable_irqs();
                    let syscall_id = tf.regs.a7;

                    debug!("[user_task_top]sys_call start syscall id = {} tid = {},pid={},sepc:{:#x},a0:{}",
                     syscall_id,curr.id(),curr.get_process().unwrap().get_pid(),sepc,tf.regs.a0);

                    let args = [
                        tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3, tf.regs.a4, tf.regs.a5,
                    ];

                    tf.sepc += 4;

                    let result = syscall(syscall_id, args).await;

                    curr.update_stime();
                    let result = match result {
                        Ok(res) => res,
                        Err(err) => {
                            if err == SysErrNo::EAGAIN {
                                tf.sepc -= 4;
                                debug!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);

                                yield_now().await;

                                tf.regs.a0
                            } else if err == SysErrNo::ECHILD {
                               
                                debug!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);


                                -(err as isize) as usize
                            } else if err == SysErrNo::EINVAL {
                                warn!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);

                                -(err as isize) as usize
                                
                            } else {
                                warn!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);
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
                }
                Trap::Exception(Exception::StorePageFault)
                | Trap::Exception(Exception::PageModifyFault) |
                Trap::Exception(Exception::LoadPageFault)
        | Trap::Exception(Exception::PageNonReadableFault)=>
                 {
                    //懒分配
                    // println!(
                    //     "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
                    //     scause.cause(),
                    //     stval,
                    //     sepc
                    // );
                    let is_write = match scause.cause() {
                        Trap::Exception(Exception::StorePageFault) 
                        | Trap::Exception(Exception::PageModifyFault) => true,
                        _ => false,
                    };
                    
                   
                    let handleres= curr
                        .get_process()
                        .unwrap()
                        .memory_set
                        .lock().await
                        .handle_page_fault(stval,is_write).await;
                    match handleres {
                        Ok(value) if value == false => {
                            exit_proc(-2).await;
                            log_page_fault_error(scause, stval, sepc);
                        }
                        Err(_) => {
                            exit_proc(-2).await;
                            log_page_fault_error(scause, stval, sepc);
                        }
                        _ => {}
                    }
                }
            //     Trap::Exception(Exception::StoreFault)
            //     | Trap::Exception(Exception::InstructionFault)
            //     | Trap::Exception(Exception::LoadFault) => {
            //         println!(
            //     "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it....",
            //     scause.cause(),
            //     stval,
            //     tf.sepc ,
            // );

            //         // page fault exit code

            //         exit_current(-2).await;
            //     }
                
                Trap::Exception(Exception::Breakpoint) => {
                    println!("[kernel] Breakpoint exception in application (sepc={:#x}). Probably from abort(). Terminating process.", tf.sepc);
                    let task= current_task();
                    let cx = task.get_trap_cx().unwrap();
                     cx.sepc += 2;
                }
                Trap::Exception(Exception::PageNonExecutableFault)
                | Trap::Exception(Exception::FetchPageFault) =>{
                    println!("[kernel] Illegal instruction exception in application (sepc={:#x}). Probably from abort(). Terminating process.", tf.sepc);

                    exit_current(-2).await;
                }
                

                Trap::Interrupt(_) => {

                        /// Timer IRQ of loongarch64
                       const TIMER_IRQ: usize = 11;
                    let irq_num: usize = loongArch64::register::estat::read().is().trailing_zeros() as usize;
                    match irq_num {
                        // TIMER_IRQ
                        TIMER_IRQ => {
                            loongArch64::register::ticlr::clear_timer_interrupt();
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

                        }
                        _ => panic!("unknown interrupt: {}", irq_num),
                    }
                } 
                _ => {
                    panic!(
                        "Unsupported trap {:?}, stval = {:#x}!",
                        scause.cause(),
                        stval
                    );
                }
            }

            {
                //处理完系统调用过后，对应的信号处理和时钟更新
                crate::signal::handle_pending_signals(syscall_ret).await;
                crate::task::sleeplist::process_timed_events();
                crate::timer::handle_timer_tick().await;
            }

            // trace!("sys_call end3");
            // 判断任务是否退出

            
            tf.trap_status = TrapStatus::Done;
            // trace!("sys_call end4");
            if curr.is_zombie() || curr.get_process().is_none() {
                // 任务结束，需要切换至其他任务，关中断
                disable_irqs();
                // info!("return ready");
                return curr.get_exit_code() as i32;
            } else {
                if let Some(pcb) = curr.get_process() {
                    if *pcb.state.lock().await == TaskStatus::Zombie {
                        // 任务结束，需要切换至其他任务，关中断
                        disable_irqs();
                        // info!("return ready");
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
#[cfg(target_arch="riscv64")]
pub async fn user_task_top() -> i32 {
    loop {
        debug!("into user_task_top");
        let curr = current_task();

        let mut syscall_ret = None;
        let tf = curr.get_trap_cx().unwrap();
        // debug!("trap_status:{:?}",tf.trap_status);
        if tf.trap_status == TrapStatus::Blocked {
            let scause = scause::read();
            let stval = stval::read();
            let sepc = sepc::read();
      
            // if curr.get_pid()==3{
            //     trace!(
            //         "Trap:{:#?}",tf
            //     )
            // }
            match scause.cause() {
                Trap::Exception(Exception::UserEnvCall) => {
                    enable_irqs();
                    let syscall_id = tf.regs.a7;

                    debug!("[user_task_top]sys_call start syscall id = {} tid = {},pid={},sepc:{:#x},a0:{}",
                     syscall_id,curr.id(),curr.get_process().unwrap().get_pid(),sepc,tf.regs.a0);

                    let args = [
                        tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3, tf.regs.a4, tf.regs.a5,
                    ];

                    tf.sepc += 4;

                    let result = syscall(syscall_id, args).await;

                    curr.update_stime();
                    let result = match result {
                        Ok(res) => res,
                        Err(err) => {
                            if err == SysErrNo::EAGAIN {
                                tf.sepc -= 4;
                                debug!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);

                                yield_now().await;

                                tf.regs.a0
                            } else if err == SysErrNo::ECHILD {
                               
                                debug!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);


                                -(err as isize) as usize
                            } else if err == SysErrNo::EINVAL {
                                warn!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);

                                -(err as isize) as usize
                                
                            } else {
                                warn!("\x1b[93m [Syscall]Err: {},syscall:{}\x1b[0m", err.str(),tf.regs.a7);
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
                }
                Trap::Exception(Exception::StorePageFault)
                | Trap::Exception(Exception::LoadPageFault)
                | Trap::Exception(Exception::InstructionPageFault)=>
                 {
                    //懒分配
                    // println!(
                    //     "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
                    //     scause.cause(),
                    //     stval,
                    //     sepc
                    // );
                    let is_write:bool=
                        scause.cause() == Trap::Exception(Exception::StorePageFault);

                    let stval=if scause.cause() == Trap::Exception(Exception::IllegalInstruction) {
                        // 如果是非法指令异常，stval 是 sepc
                        sepc
                    } else {
                        stval
                    };
                    let handleres= curr
                        .get_process()
                        .unwrap()
                        .memory_set
                        .lock().await
                        .handle_page_fault(stval,is_write).await;
                    match handleres {
                        Ok(value) if value == false => {
                            exit_proc(-2).await;
                            log_page_fault_error(scause, stval, sepc);
                        }
                        Err(_) => {
                            exit_proc(-2).await;
                            log_page_fault_error(scause, stval, sepc);
                        }
                        _ => {}
                    }
                }
                Trap::Exception(Exception::StoreFault)
                | Trap::Exception(Exception::InstructionFault)
                | Trap::Exception(Exception::LoadFault) => {
                    println!(
                "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it....",
                scause.cause(),
                stval,
                tf.sepc ,
            );

                    // page fault exit code

                    exit_current(-2).await;
                }
                
                Trap::Exception(Exception::Breakpoint) => {
                    println!("[kernel] Breakpoint exception in application (sepc={:#x}). Probably from abort(). Terminating process.", tf.sepc);
                    let task= current_task();
                    let cx = task.get_trap_cx().unwrap();
                     cx.sepc += 2;
                }
Trap::Exception(Exception::IllegalInstruction)  =>{
                    println!("[kernel] Illegal instruction exception in application (sepc={:#x}). Probably from abort(). Terminating process.", tf.sepc);

                    exit_current(-2).await;
                }
                

                Trap::Interrupt(Interrupt::SupervisorTimer) => {
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
                }
                _ => {
                    panic!(
                        "Unsupported trap {:?}, stval = {:#x}!",
                        scause.cause(),
                        stval
                    );
                }
            }

            {
                //处理完系统调用过后，对应的信号处理和时钟更新
                crate::signal::handle_pending_signals(syscall_ret).await;
                crate::task::sleeplist::process_timed_events();
                crate::timer::handle_timer_tick().await;
            }

            // trace!("sys_call end3");
            // 判断任务是否退出

            
            tf.trap_status = TrapStatus::Done;
            // trace!("sys_call end4");
            if curr.is_zombie() || curr.get_process().is_none() {
                // 任务结束，需要切换至其他任务，关中断
                disable_irqs();
                // info!("return ready");
                return curr.get_exit_code() as i32;
            } else {
                if let Some(pcb) = curr.get_process() {
                    if *pcb.state.lock().await == TaskStatus::Zombie {
                        // 任务结束，需要切换至其他任务，关中断
                        disable_irqs();
                        // info!("return ready");
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

