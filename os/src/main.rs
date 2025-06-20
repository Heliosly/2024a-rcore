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
#![deny(missing_docs)]
#![no_std]
#![no_main]
#![feature(panic_info_message)]
#![feature(alloc_error_handler)]

extern crate alloc;
extern crate bitflags;
#[macro_use]
extern crate log;

#[macro_use]
mod console;

pub mod config;
pub mod arch;  // 架构抽象层
pub mod drivers;
pub mod fs;
pub mod lang_items;
pub mod logging;
pub mod mm;
#[cfg(target_arch = "riscv64")]
pub mod sbi;
pub mod sync;
pub mod syscall;
pub mod task;
pub mod timer;
pub mod trap;
pub mod utils;

use core::arch::{asm, global_asm};
use alloc::boxed::Box;
use config::KERNEL_DIRECT_OFFSET;
use trap::user_task_top;

#[cfg(target_arch = "riscv64")]
global_asm!(include_str!("entry.asm"));

#[cfg(target_arch = "loongarch64")]
global_asm!(include_str!("arch/loongarch64/entry.asm"));

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

#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn setbootsp() {
    unsafe {
        asm!("add sp, sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
        asm!("la t0, rust_main");
        asm!("add t0, t0, {}", in(reg) KERNEL_DIRECT_OFFSET );
        asm!("jalr zero, 0(t0)");
    }
}

#[cfg(target_arch = "loongarch64")]
#[no_mangle]
pub fn setbootsp() {
    unsafe {
        asm!("addi.d $sp, $sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
        asm!("la.global $t0, rust_main");
        asm!("add.d $t0, $t0, {}", in(reg) KERNEL_DIRECT_OFFSET );
        asm!("jirl $zero, $t0, 0");
    }
}

#[no_mangle]
pub fn rust_main() -> ! {
    clear_bss();
    
    arch_init();
    
    logging::init();
    mm::init();
    
    trap::init();
    
    timer::init();
    task::run_tasks();
}

fn arch_init() {
    #[cfg(target_arch = "riscv64")]
    {
    }
    
    #[cfg(target_arch = "loongarch64")]
    {
        // LoongArch 特定初始化
        crate::arch::loongarch64::trap::init_trap();
    }
}
