//! The main module and entrypoint
//!
//! Various facilities of the kernels are implemented as submodules. The most
//! important ones are:
//!
//! - [`trap`]: Handles all cases of switching from userspace to the kernel
//! - [`task`]: Task management
//! - [`syscall`]: System call handling and implementation
//! - [`mm`]: Address map using SV39
//! - [`sync`]: Wrap a static data structure inside it so that we are able to access it without any `unsafe`.
//! - [`fs`]: Separate user from file system with some structures
//!
//! The operating system also starts in this module. Kernel code starts
//! executing from `entry.asm`, after which [`rust_main()`] is called to
//! initialize various pieces of functionality. (See its source code for
//! details.)
//!
//! We then call [`task::run_tasks()`] and for the first time go to
//! userspace.
#![allow(missing_docs)]
#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![feature(linked_list_retain)]
#![feature(linked_list_cursors)]
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;

extern crate alloc;

#[macro_use]
mod console;

pub mod config;
pub mod drivers;
pub mod fs;
pub mod lang_items;
pub mod logging;
pub mod mm;
pub mod sbi;
pub mod sync;
pub mod syscall;
pub mod task;
pub mod timer;
// pub mod executor;

pub mod signal;
pub mod trap;
///utils;

pub mod utils;
use core::arch::{asm, global_asm};
use alloc::boxed::Box;
use config::KERNEL_DIRECT_OFFSET;
use trap::user_task_top;

global_asm!(include_str!("entry.asm"));
global_asm!(include_str!("signal.S"));
/// clear BSS segment
fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    unsafe {
        core::slice::from_raw_parts_mut(sbss as usize as *mut u8, ebss as usize - sbss as usize)
            .fill(0);
    }

}

#[no_mangle]
///立即数高于12位用rust处理
pub fn setbootsp() {
   unsafe {
        asm!("add sp, sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
        asm!("la t0, rust_main");
        asm!("add t0, t0, {}", in(reg) KERNEL_DIRECT_OFFSET );
        asm!("jalr zero, 0(t0)");
       
    }
}

#[no_mangle]
/// the rust entry-point of os
pub fn rust_main() -> ! {
    clear_bss();
    println!("[kernel] Hello, !");
    
    logging::init();
    mm::init();
    // mm::heap_allocator::heap_test();
    // mm::frame_allocator::frame_allocator_test();
    trap::init();
    trap::enable_irqs();
    timer::set_next_trigger();
    task::init(|| Box::pin(user_task_top()));

    fs::init();
    // fs::list_app();
    
    // task::add_initproc("/", "/glibc/busybox",  "sh /initproc.sh");

    //  task::add_initproc("/basic", "/basic/sigtest", "");

    // task::add_initproc("/glibc", "/glibc/busybox", "sh run-dynamic.sh");

    task::add_initproc("/glibc", "/glibc/busybox", "sh run-static.sh");

    //  task::add_initproc("/disk", "/glibc/busybox", "sh /disk/run-dynamic.sh");
    //  task::add_initproc("/libctest", "/glibc/busybox", "sh /libctest/run-static.sh");

    extern  "C" {
        fn trampoline(tc: usize, has_trap: bool, from_user: bool) -> !;
    }

    unsafe {
        trampoline(0, false, false);
    }
}


#[no_mangle]
pub static mut __stack_chk_guard: usize = 0xdead_beef_dead_beef;

// 栈溢出检测失败时调用的函数
#[no_mangle]
pub extern "C" fn __stack_chk_fail() {
   
  
  panic!("stack overflow detected");
   
}
