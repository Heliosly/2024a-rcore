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

mod context;
// use crate::mm::activate_kernel_space;
//use crate::config:: TRAP_CONTEXT_BASE;
use crate::syscall::syscall;
use crate::task::{
    current_trap_cx,exit_current_and_run_next, suspend_current_and_run_next,
};
use crate::utils::backtrace;
use crate::timer::set_next_trigger;
use core::arch:: global_asm;
use riscv::register::{satp, sepc, sstatus};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, stval, stvec,
    mstatus::FS,
};

global_asm!(include_str!("trap.S"));

/// Initialize trap handling
pub fn init() {
    set_kernel_trap_entry();
    unsafe {
        sstatus::set_fs(FS::Clean);
    }
}
extern "C" {
    fn __trap_from_user();
}
fn set_kernel_trap_entry() {
    unsafe {
        stvec::write(trap_from_kernel as usize, TrapMode::Direct);
    }

        trace!("stvec_kernel:{:#x},true adress :{:#x}",stvec::read().bits(),trap_from_kernel as usize);
}

fn set_user_trap_entry() {
    unsafe {
        stvec::write(__trap_from_user as usize, TrapMode::Direct);
    }

        trace!("stvec_user:{:#x},true adress :{:#x}",stvec::read().bits(),__trap_from_user as usize);
}

/// enable timer interrupt in supervisor mode
pub fn enable_timer_interrupt() {
    unsafe {
        sie::set_stimer();
    }
}

/// trap handler
#[no_mangle]
pub fn trap_handler() {
    set_kernel_trap_entry();
    trace!("trap_handler");

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
            // jump to next instruction anyway
            let mut cx = current_trap_cx();
            cx.sepc += 4;
            // get system call return value
            let result = syscall(cx.x[17], [cx.x[10], cx.x[11], cx.x[12], cx.x[13]]);
            // cx is changed during sys_exec, so we have to call it again
            cx = current_trap_cx();
            cx.x[10] = result as usize;
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
                current_trap_cx().sepc,
            );
            loop {
               
            }
            // page fault exit code

            // exit_current_and_run_next(-2);
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            println!("[kernel] IllegalInstruction in application, kernel killed it.");
            // illegal instruction exit code
            exit_current_and_run_next(-3);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            set_next_trigger();
            suspend_current_and_run_next();
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
    
}
///方便调试
#[no_mangle]
pub fn trap_loop() {
    loop {
        trap_return();
        trap_handler();
    }
}
#[no_mangle]
/// return to user space

pub fn trap_return()   {
    // set_user_trap_entry();
    // let trap_cx_ptr = TRAP_CONTEXT_BASE;
    // let user_satp = current_user_token();
    // extern "C" {
    //     fn __alltraps();
    //     fn __restore();
    // }
    // let restore_va = __restore as usize - __alltraps as usize ;
    // // trace!("[kernel] trap_return: ..before return");
    // unsafe {
    //     asm!(
    //         "fence.i",
    //         "jr {restore_va}",
    //         restore_va = in(reg) restore_va,
    //         in("a0") trap_cx_ptr,
    //         in("a1") user_satp,
    //         options(noreturn)
    //     );
    // }
    set_user_trap_entry();
    extern "C" {
        #[allow(improper_ctypes)]
        fn __return_to_user(cx: *mut TrapContext);
    }
    unsafe {
        // 方便调试进入__return_to_user
        let trap_cx = current_trap_cx();
        __return_to_user(trap_cx);
    }
}

#[no_mangle]
/// handle trap from kernel
/// Unimplement: traps/interrupts/exceptions from kernel mode
/// Todo: Chapter 9: I/O device
#[link_section = ".text.trap_entries"]

pub fn trap_from_kernel() -> ! {
    backtrace();
    let stval = stval::read();
    let sepc = sepc::read();
    // let stval_vpn = VirtPageNum::from(stval);
    // let sepc_vpn = VirtPageNum::from(sepc);
    panic!(
        "stval = {:#x}, sepc = {:#x},
        a trap {:?} from kernel",
        stval,
        
        sepc,
        
        scause::read().cause()
    );
}

pub use context::TrapContext;
