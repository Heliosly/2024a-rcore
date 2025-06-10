//! Trap handling functionality
//!
//! For rCore, we have a single trap entry point, namely `__alltraps`. At
//! initialization in [`init()`], we set the `stvec` CSR to point to it.
//!
//! All traps go through `__alltraps`, which is defined in `trap.S`. The
//! assembly language code does just enough work restore the kernel space
//! context, ensuring that Rust code safely runs, and transfers control to
//! [`trap_handler()`].
//!
//! It then calls different functionality based on what exactly the exception
//! was. For example, timer interrupts trigger task preemption, and syscalls go
//! to [`syscall()`].
// use crate::mm::activate_kernel_space;
//use crate::config:: TRAP_CONTEXT_BASE;

mod context;
mod ucontext;
use crate::config::PAGE_SIZE;
use crate::fs::File;
use crate::mm::{flush_tlb, translated_byte_buffer, MemorySet,  VirtAddr};
use crate::sync::Mutex;
use crate::syscall::syscall;
use crate::task::{
    current_process, current_task, current_task_may_uninit, exit_current, pick_next_task, run_task2, task_count, task_tick, yield_now, CurrentTask
};
use crate::timer::set_next_trigger;
use crate::utils::{bpoint, bpoint1};
use crate::utils::error::{GeneralRet, SysErrNo};
pub use context::user_return;

pub use context::TrapStatus;
use core::arch::global_asm;
use core::future::poll_fn;
use core::panic;
use core::task::Poll;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_SET};
use riscv::register::scause::Scause;
use riscv::register::{
    mstatus::FS,
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, stval, stvec,
};
use riscv::register::{satp, sepc, sstatus};
pub use ucontext::{MContext, UContext};
global_asm!(include_str!("trap.S"));

/// Initialize trap handling
pub fn init() {
    set_trap_entry();
    unsafe {
        sstatus::set_fs(FS::Clean);
    }
}

extern "C" {
    fn trap_vector_base();
}

pub fn set_sum(){
    unsafe { sstatus::set_sum() };
}
fn set_trap_entry() {
    unsafe {
        stvec::write(trap_vector_base as usize, TrapMode::Direct);
    }
}
// fn set_kernel_trap_entry() {
//     unsafe {
//         stvec::write(trap_from_kernel as usize, TrapMode::Direct);
//     }

//         trace!("stvec_kernel:{:#x},true adress :{:#x}",stvec::read().bits(),trap_from_kernel as usize);
// }

// fn set_user_trap_entry() {
//     unsafe {
//         stvec::write(__trap_from_user as usize, TrapMode::Direct);
//     }

//         trace!("stvec_user:{:#x},true adress :{:#x}",stvec::read().bits(),__trap_from_user as usize);
// }
///irq handle

/// enable timer interrupt in supervisor mode
pub fn enable_irqs() {
    unsafe {
        sie::set_stimer();
    }
}
/// disable timer interrupt in supervisor mode
pub fn disable_irqs() {
    unsafe {
        sie::clear_stimer();
    }
}
/// 开启内核中断
#[inline]
pub fn enable_kernel_irqs() {
    unsafe { sstatus::set_sie() }
}

///  关闭内核中断
#[inline]
pub fn disable_kernel_irqs() {
    unsafe { sstatus::clear_sie() }
}
/// Relaxes the current CPU and waits for interrupts.
///
/// It must be called with interrupts enabled, otherwise it will never return.
#[inline]
pub fn wait_for_irqs() {
    unsafe { riscv::asm::wfi() }
}

#[no_mangle]
/// Unimplement: traps/interrupts/exceptions from kernel mode

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

pub use context::TrapContext;
/// 进入 Trampoline 的方式：
///   1. 初始化后函数调用：没有 Trap，但存在就绪任务
///   2. 内核发生 Trap：存在任务被打断（CurrentTask 不为空），或者没有任务被打断（CurrentTask 为空）
///   3. 用户态发生 Trap：任务被打断，CurrentTask 不为空
///
/// 内核发生 Trap 时，将 TrapFrame 保存在内核栈上
/// 在用户态发生 Trap 时，将 TrapFrame 直接保存在任务控制块中，而不是在内核栈上
///
/// 只有通过 trap 进入这个入口时，是处于关中断的状态，剩下的任务切换是没有关中断
#[no_mangle]
pub fn trampoline(_tc: *mut TrapContext, has_trap: bool, from_user: bool) {
    loop {
        if !from_user && has_trap {
            // 在内核中发生了 Trap，只处理中断，目前不支持抢占
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
                    let res= CurrentTask::get();
                    // trace!("take a task tid = {}",res.id());

                    Some(res)
                } else {
                    None
                }
            }) {
                //  debug!("run_task pid:{},Arc count:{}",curr.pid.0,

                //                    Arc::strong_count(&curr)
                //                        );

                    trace!("run task tid = {}",curr.id());
                run_task2(CurrentTask::from(curr));
            } else {
                enable_irqs();
                // warn!("no tasks available in run_tasks");

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
            trace!(
                "Trap: cause={:?}, addr={:#x}, sepc={:#x}, satp={:#x} ",
                scause.cause(),
                stval,
                sepc,
                satp::read().bits()
            );
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
                     syscall_id,curr.id(),curr.get_process().get_pid(),sepc,tf.regs.a0);
                    
                    let args = [
                        tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3, tf.regs.a4, tf.regs.a5,
                    ];

                    tf.sepc += 4;

                    let result = syscall(syscall_id, args).await;

                    curr.update_stime();
                    let result = match result {
                        Ok(res) => res,
                        Err(err) => {

                            if err ==SysErrNo::EAGAIN{
                                tf.sepc-=4;
                               bpoint();
                               debug!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());

                               yield_now().await;

                                tf.regs.a0
                 
                            }
                          else if  err==SysErrNo::ECHILD{
                               debug!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());

                                -(err as isize) as usize
                            }
                            else if err == SysErrNo::EINVAL{
                                println!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                                -(err as isize) as usize
                            }
                            else{
                            warn!("\x1b[93m [Syscall]Err: {}\x1b[0m", err.str());
                            -(err as isize) as usize
                            
                            }
                              
                            // debug!("[Syscall]Err:{}", err.str());

                        }
                    };
            
                    syscall_ret =  Some(result);

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
                | Trap::Exception(Exception::InstructionPageFault) => {
                    //懒分配
                    // println!(
                    //     "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
                    //     scause.cause(),
                    //     stval,
                    //     sepc
                    // );
                   if curr.get_process().memory_set.lock().await.  handle_page_fault(  stval).await.is_err(){
                       exit_current(-2).await; log_page_fault_error(scause, stval, sepc)
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
                Trap::Exception(Exception::IllegalInstruction) => {
                    println!("[kernel] IllegalInstruction in application, kernel killed it.");
                    // illegal instruction exit code
                    exit_current(-3).await;
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
                                "[user_task_top]current {} is to be preempted in user mode, allow {}",
                                curr.id(),
                                curr.can_preempt()
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
            
            tf.trap_status = TrapStatus::Done;
           {
                //处理完系统调用过后，对应的信号处理和时钟更新
                crate::signal::handle_pending_signals().await;
                crate::task::sleeplist::process_timed_events();
            }

            if let Some(res) = syscall_ret{

                 tf.set_arg0(res);
            }
            // trace!("sys_call end3");
            // 判断任务是否退出

            // trace!("sys_call end4");
            if curr.is_zombie() {
                // 任务结束，需要切换至其他任务，关中断
                disable_irqs();
                // info!("return ready");
                return curr.get_exit_code() as i32;
            }
            disable_irqs();
        }

        // trace!("sys_call end5");

        //偶数次poll时会从重新运行这个函数开始
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
