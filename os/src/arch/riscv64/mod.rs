use core::arch::global_asm;
use core::arch::asm;

global_asm!(include_str!("entry.asm"));
global_asm!(include_str!("signal.S"));

pub mod trap;
// pub mod context;
// pub mod mm;
// pub mod timer;

use crate::arch::ArchInit;
use crate::config::KERNEL_DIRECT_OFFSET;
use super::TrapArch;
pub struct RiscV64;

impl ArchInit for RiscV64 {
    fn arch_init() {
        trap::RiscV64Trap::init_trap();
    }
    
    fn set_boot_stack() {
        unsafe {
            asm!("add sp, sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
        }
    }
    
    fn jump_to_rust_main() -> ! {
        unsafe {
            asm!("la t0, rust_main");
            asm!("add t0, t0, {}", in(reg) KERNEL_DIRECT_OFFSET);
            asm!("jalr zero, 0(t0)");
        }
        unreachable!()
    }
}

#[no_mangle]
pub fn setbootsp() -> ! {
    RiscV64::set_boot_stack();
    RiscV64::jump_to_rust_main();
}
