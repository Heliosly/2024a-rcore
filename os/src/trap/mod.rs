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
use crate::syscall:: syscall;
use crate::task::{
    current_task, current_task_trapctx_ptr, exit_current_and_run_next, pick_next_task, run_task2, task_tick, CurrentTask
};
pub use context::user_return;
use alloc::sync::Arc;
pub use context::TrapStatus;
use crate::utils::backtrace;
use crate::timer::set_next_trigger;
use core::arch:: global_asm;
use core::future::poll_fn;
use core::panic;
use core::task::Poll;
use riscv::register::{satp, sepc, sstatus};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, stval, stvec,
    mstatus::FS,
};

#[cfg(target_arch = "riscv64")]
global_asm!(include_str!("../arch/riscv64/trap.S"));

#[cfg(target_arch = "loongarch64")]
global_asm!(include_str!("../arch/loongarch64/trap.S"));

use crate::arch::CurrentArch;

#[cfg(target_arch = "loongarch64")]
global_asm!(include_str!("trap_loongArch.S"));

extern "C" {
    fn trap_return1();
    fn user_return1();
}

/// Initialize trap handling
pub fn init() {
    CurrentArch::set_trap_entry(trap_vector_base as usize);
}

fn set_trap_entry(){
    CurrentArch::set_trap_entry(trap_vector_base as usize);
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
    crate::arch::enable_irqs();
}

/// disable timer interrupt in supervisor mode
pub fn disable_irqs() {
    crate::arch::disable_irqs();
}
#[cfg(target_arch = "loongarch64")]
pub fn disable_irqs() {
    unsafe {
        core::arch::asm!("csrwr {}, 0x4", in(reg) 0x0);
    }
}

/// 开启内核中断
#[inline]
pub fn enable_kernel_irqs() {
    crate::arch::enable_kernel_irqs();
}

/// 关闭内核中断
#[inline]
pub fn disable_kernel_irqs() {
    crate::arch::disable_kernel_irqs();
}

/// Relaxes the current CPU and waits for interrupts.
///
/// It must be called with interrupts enabled, otherwise it will never return.
#[inline]
pub fn wait_for_irqs() {
    crate::arch::wait_for_irqs();
}

#[no_mangle]
/// Unimplement: traps/interrupts/exceptions from kernel mode

pub fn trap_from_kernel() {
    backtrace();
    let stval = stval::read();
    let sepc = sepc::read();
    // let stval_vpn = VirtPageNum::from(stval);
    // let sepc_vpn = VirtPageNum::from(sepc);
    let scause = scause::read();
    match scause.cause()  {
        Trap::Interrupt(Interrupt::SupervisorTimer)=>{

        }
        _ =>{
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
pub fn trampoline(_tc: &mut TrapContext, has_trap: bool, from_user: bool) {
    loop {
        if !from_user && has_trap {
            // 在内核中发生了 Trap，只处理中断，目前不支持抢占
            trap_from_kernel();
            return;
        } else {
            // debug!("into trampoline from taskcount:{},task",get_task_count());
            // 用户态发生了 Trap 或者需要调度
            if let Some(curr) = current_task().or_else(|| {
                if let Some(task) = pick_next_task() {
                    unsafe {
                        CurrentTask::init_current(task.clone());
                         
                        //  debug!("get and init next task");

                    }
                    Some(task)
                } else {
                    None
                }
            }) {
 debug!("run_task pid:{},Arc count:{}",curr.pid.0,
                  
                   Arc::strong_count(&curr)
                       );
                run_task2(CurrentTask::from(curr));
            } else {
                enable_irqs();
                debug!("no tasks available in run_tasks");

                wait_for_irqs();
            }
        }
    }
}
///a future to handle user trap
/// IMPO

pub async fn user_task_top() -> i32 {
    loop {

        // debug!("into user_task_top");
        let curr = current_task().unwrap();
        let inner = curr.inner_exclusive_access();
        let  tf =  inner.trap_cx.get();
        drop(inner);
        // debug!("trap_status:{:?}",tf.trap_status);
        if (unsafe { (*tf) .trap_status })== TrapStatus::Blocked {
            let scause = scause::read();
    let stval = stval::read();
    let sepc = sepc::read();
    trace!(
        "Trap: cause={:?}, addr={:#x}, sepc={:#x}, satp={:#x}",
        scause.cause(),
        stval,
        sepc,
        satp::read().bits()
    );
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            enable_irqs();
            let syscall_id;
            let args;
unsafe {
             syscall_id = (*tf).regs.a7;
 args = [(*tf).regs.a0, (*tf).regs.a1, (*tf).regs.a2, (*tf).regs.a3];

(*tf).sepc += 4;

}
let result = syscall(syscall_id, args).await;

// trace!("sys_call end");

    let tf = current_task_trapctx_ptr();
           unsafe {
               (*tf).regs.a0 = result as usize;
           } 

// trace!("sys_call end1");
 // 判断任务是否退出
 let curr = current_task().unwrap();
 if curr.is_exited() {
    // 任务结束，需要切换至其他任务，关中断
    disable_irqs();
    return curr.get_exit_code() as i32;
}

disable_irqs();
        }
        Trap::Exception(Exception::StoreFault)
        | Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::InstructionFault)
        | Trap::Exception(Exception::InstructionPageFault)
        | Trap::Exception(Exception::LoadFault)
        | Trap::Exception(Exception::LoadPageFault) => {
            
            println!(
                "[kernel] trap_handler:  {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, kernel killed it.",
                scause.cause(),
                stval,
                unsafe { (*tf).sepc },
            );
           
            // page fault exit code

            exit_current_and_run_next(-2);
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            println!("[kernel] IllegalInstruction in application, kernel killed it.");
            // illegal instruction exit code
            exit_current_and_run_next(-3);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            set_next_trigger();
            
            unsafe {(*tf).trap_status=TrapStatus::Done};
            task_tick(current_task().unwrap());
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
           
           let tf=current_task_trapctx_ptr();

            unsafe { (*tf).trap_status = TrapStatus::Done };
            // trace!("sys_call end3");
            // 判断任务是否退出
            let curr= current_task().unwrap();
            let inner =curr.inner_exclusive_access();

            // trace!("sys_call end4");
            if inner.is_zombie() {
                // 任务结束，需要切换至其他任务，关中断
                disable_irqs();
                // info!("return ready");
                return inner.exit_code ;
            }
            disable_irqs();
            drop(inner);
        }

            // trace!("sys_call end5");
        
           let tf=current_task_trapctx_ptr();
        //偶数次poll时会从重新运行这个函数开始
        poll_fn(|_cx| {
            if (unsafe { (*tf) .trap_status })== TrapStatus::Done {
                Poll::Pending
            } else {

                Poll::Ready(0)

            }
        })
        .await ;
  
    }
}

#[cfg(target_arch = "loongarch64")]
fn handle_loongarch_exception(tf: &mut TrapContext) {
    use crate::trap::context::loongarch_csr::*;
    
    // 读取 LoongArch 的 CSR
    let estat: usize;
    let era: usize;
    let badv: usize;
    let crmd: usize;
    
    unsafe {
        core::arch::asm!(
            "csrrd {}, {}",
            out(reg) estat,
            const CSR_ESTAT,
        );
        core::arch::asm!(
            "csrrd {}, {}",
            out(reg) era,
            const CSR_ERA,
        );
        core::arch::asm!(
            "csrrd {}, {}",
            out(reg) badv,
            const CSR_BADV,
        );
        core::arch::asm!(
            "csrrd {}, {}",
            out(reg) crmd,
            const CSR_CRMD,
        );
    }
    
    // 提取异常代码 (ESTAT[16:21])
    let ecode = (estat >> 16) & 0x3f;
    let esubcode = (estat >> 22) & 0x1ff;
    
    trace!(
        "LoongArch Trap: ecode={:#x}, esubcode={:#x}, era={:#x}, badv={:#x}",
        ecode, esubcode, era, badv
    );
    
    match ecode {
        0x0b => {
            // 系统调用异常 (SYSCALL)
            enable_irqs();
            let syscall_id = tf.regs.a7;
            let args = [tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3];
            
            tf.sepc = era + 4;
            
            // 处理系统调用（这里需要适配为异步版本）
            // let result = syscall(syscall_id, args).await;
            // tf.regs.a0 = result as usize;
            
            // 注意：这里需要重构为适配异步系统调用的版本
            println!("LoongArch syscall: id={}, args=[{:#x}, {:#x}, {:#x}, {:#x}]", 
                    syscall_id, args[0], args[1], args[2], args[3]);
        }
        
        0x01 => {
            // TLB 重填异常
            println!(
                "[kernel] LoongArch TLB Refill: badv={:#x}, era={:#x}",
                badv, era
            );
            exit_current_and_run_next(-2);
        }
        
        0x02 => {
            // TLB 无效异常
            println!(
                "[kernel] LoongArch TLB Invalid: badv={:#x}, era={:#x}",
                badv, era
            );
            exit_current_and_run_next(-2);
        }
        
        0x03 => {
            // TLB 修改异常
            println!(
                "[kernel] LoongArch TLB Modified: badv={:#x}, era={:#x}",
                badv, era
            );
            exit_current_and_run_next(-2);
        }
        
        0x04 => {
            // 地址错误异常（取指）
            println!(
                "[kernel] LoongArch Address Error (Fetch): badv={:#x}, era={:#x}",
                badv, era
            );
            exit_current_and_run_next(-2);
        }
        
        0x05 => {
            // 地址错误异常（访存）
            println!(
                "[kernel] LoongArch Address Error (Memory): badv={:#x}, era={:#x}",
                badv, era
            );
            exit_current_and_run_next(-2);
        }
        
        0x06 => {
            // 指令错误异常
            println!(
                "[kernel] LoongArch Instruction Error: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x07 => {
            // 权限等级错误异常
            println!(
                "[kernel] LoongArch Privilege Error: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x08 => {
            // 浮点指令异常
            println!(
                "[kernel] LoongArch Float Point Exception: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x09 => {
            // 断点异常
            println!(
                "[kernel] LoongArch Breakpoint: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x0a => {
            // 保留指令异常
            println!(
                "[kernel] LoongArch Reserved Instruction: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x0c => {
            // 机器错误异常
            println!(
                "[kernel] LoongArch Machine Error: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        0x0d => {
            // 算术溢出异常
            println!(
                "[kernel] LoongArch Arithmetic Overflow: era={:#x}",
                era
            );
            exit_current_and_run_next(-3);
        }
        
        _ => {
            // 未知异常
            panic!(
                "Unsupported LoongArch exception: ecode={:#x}, esubcode={:#x}, era={:#x}, badv={:#x}",
                ecode, esubcode, era, badv
            );
        }
    }
    
    tf.trap_status = TrapStatus::Done;
    disable_irqs();
}

#[cfg(target_arch = "loongarch64")]
async fn handle_loongarch_syscall(tf: &mut TrapContext) -> isize {
    let syscall_id = tf.regs.a7;
    let args = [tf.regs.a0, tf.regs.a1, tf.regs.a2, tf.regs.a3];
    tf.sepc += 4;
    syscall(syscall_id, args).await
}
