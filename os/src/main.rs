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
#![feature(used_with_arg)]
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate log;

extern crate alloc;

#[macro_use]
mod console;
pub mod devices;
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
pub mod arch;
// pub mod executor;

pub mod signal;
pub mod trap;
///utils;

pub mod utils;

// use core::arch::{asm, global_asm};
use alloc::boxed::Box;
use polyhal::PhysAddr;
use trap::user_task_top;

use crate::{config::PAGE_SIZE, fs::{open_file, OpenFlags}, mm::{frame_allocator::{frame_alloc_persist, frame_dealloc_persist}, frame_dealloc}};
use polyhal_boot::define_entry;
// global_asm!(include_str!("entry.asm"));
/// clear BSS segment
fn clear_bss() {
    extern "C" {
        fn _sbss();
        fn _ebss();
    }
    unsafe {
        core::slice::from_raw_parts_mut(_sbss as usize as *mut u8, _ebss as usize -_sbss as usize)
            .fill(0);
    }

}
pub struct PageAllocImpl;

impl polyhal::common::PageAlloc for PageAllocImpl {
    #[inline]
    fn alloc(&self) -> PhysAddr {
        unsafe {PhysAddr::new( frame_alloc_persist().expect("can't alloc frame")) }
    }

    #[inline]
    fn dealloc(&self, paddr: PhysAddr) {
        unsafe {
            frame_dealloc_persist(paddr.raw());
            paddr.clear_len(PAGE_SIZE);
        }
    }
}

// #[export_name = "_interrupt_for_arch"]
// /// Handle kernel interrupt
// fn kernel_interrupt(cx_ref: &mut polyhal_trap::trapframe::TrapFrame, trap_type: polyhal_trap::trap::TrapType) {
//             warn!("trap_type: {:?}  ", trap_type);
//     match trap_type {
     
//         TrapType::Timer => {
//             polyhal::timer::set_next_timer(polyhal::timer::current_time() + Duration::from_millis(10));
//         }
        
//         TrapType::SupervisorExternal => {
//             handle_external_interrupt();
//         }
//         _ => {
            
//             debug!("kernel_interrupt");
//         }
//     };
// }
// /// Handles external interrupts on LoongArch by querying the CPU's EIOI controller.
// pub fn handle_external_interrupt() {
//     // if let Some(int_driver) = devices::INT_DEVICE.try_get() {
//     //     while let Some(irq) = int_driver.claim() {
//     //         // 查找并调用注册的驱动处理程序
//     //         let handler = drivers::IRQ_HANDLERS.lock().get(&irq).cloned();
            
//     //         if let Some(driver) = handler {
//     //             // driver.handle_irq();
//     //         } else {
//     //             warn!("Unhandled IRQ: {}", irq);
//     //         }
            
//     //         int_driver.complete(irq);
//     //     }
//     // }
// }


// #[no_mangle]
// ///立即数高于12位用rust处理
// pub fn setbootsp() {
//    unsafe {
//         asm!("add sp, sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
//         asm!("la t0, rust_main");
//         asm!("add t0, t0, {}", in(reg) KERNEL_DIRECT_OFFSET );
//         asm!("jalr zero, 0(t0)");
       
//     }
// }
/// the rust entry-point of os
/// 
pub fn main(hart_id:usize) -> ! {
    #[cfg(target_arch="riscv64")]
    clear_bss();
    println!("[kernel] Hello, !");
    polyhal::irq::IRQ::int_disable();
    println!("dmw1:{:#x},dmw0 :{:#x}",loongArch64::register::dmw1::read().raw(),loongArch64::register::dmw0::read().raw());
    logging::init();
    polyhal::common::init(&PageAllocImpl);

    
    trap::init();
    mm::init();
    mm::remap_test();
    mm::heap_allocator::heap_test();
    mm::frame_allocator::frame_allocator_test();
    trap::enable_irqs();
    timer::set_next_trigger();
    timer::init_timer_backend();
    task::init(|| Box::pin(user_task_top()));


    fs::init();
    // fs::list_app();
    
    // task::add_initproc("/", "/musl/busybox",  "sh /initproc.sh");

    // task::add_initproc("/", "/musl/busybox",  "sh /write_tmp.sh");
    //  task::add_initproc("/basic", "/basic/sigtest", "");


    //  task::add_initproc("/glibc", "/musl/busybox", "sh cyclictest_testcode.sh");
    // task::add_initproc("/musl", "/musl/busybox", "sh run-dynamic.sh");

    // task::add_initproc("/musl", "/argexe", "sh run-dynamic.sh sadsadagg");
    // task::add_initproc("/glibc", "/glibc/busybox", "sh run-static.sh");
    // task::add_initproc("/musl", "/musl/busybox", "sh");

    // task::add_initproc("/glibc", "/glibc/busybox", "sh");
    // task::add_initproc("/musl", "/musl/busybox", "sh run-dynamic.sh");

    //  task::add_initproc("/musl", "/musl/busybox", "sh /musl/run-static.sh");

     task::add_initproc("/musl", "/musl/basic/openat", "");
    //  task::add_initproc("/glibc", "/glibc/basic/mmap", "");

    //  task::add_initproc("/glibc", "/glibc/busybox", "sh basic_testcode.sh");
    //  task::add_initproc("/musl", "/musl/busybox", "sh basic_testcode.sh");
    //  task::add_initproc("/musl", "/musl/busybox", "sh /musl/run-static.sh");
    //  task::add_initproc("/libctest", "/glibc/busybox", "sh /libctest/run-static.sh");
    // open_file("/usr/lib", OpenFlags::O_PATH,0).unwrap();
    extern  "C" {
        fn trampoline(tc: usize, has_trap: bool, from_user: bool) -> !;
    }

    unsafe {
        trampoline(0, false, false);
    }
}

#[no_mangle]
pub static mut __stack_chk_guard: usize = 0xdead_beef_aaad_beef;
define_entry!(main);
// 栈溢出检测失败时调用的函数
#[no_mangle]
pub extern "C" fn __stack_chk_fail() {
  panic!("stack overflow detected");
}
