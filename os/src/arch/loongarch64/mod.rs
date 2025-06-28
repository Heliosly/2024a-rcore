use core::arch::global_asm;
use core::arch::asm;

// global_asm!(include_str!("entry.asm"));
global_asm!(include_str!("signal.S"));

pub mod trap;
// pub mod context;
// pub mod mm;
// pub mod timer;

use crate::arch::ArchInit;
use crate::config::KERNEL_DIRECT_OFFSET;
use crate::arch::TrapArch;

pub struct LoongArch64;
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[no_mangle]
pub extern "C" fn _Unwind_Resume() -> ! {
    panic!("_Unwind_Resume called");
}
impl ArchInit for LoongArch64 {
    fn arch_init() {
        trap::LoongArch64Trap::init_trap();
    }
    
    fn set_boot_stack() {
        unsafe {
            asm!("add.d $sp, $sp, {}", in(reg) KERNEL_DIRECT_OFFSET);
        }
    }
    
    fn jump_to_rust_main() -> ! {
        unsafe {
            asm!("la.global $t0, rust_main");
            asm!("add.d $t0, $t0, {}", in(reg) KERNEL_DIRECT_OFFSET);
            asm!("jirl $zero, $t0, 0");
        }
        unreachable!()
    }
}

#[no_mangle]
pub fn setbootsp() -> ! {
    LoongArch64::set_boot_stack();
    LoongArch64::jump_to_rust_main();
}
#[naked]
pub unsafe extern "C" fn tlb_fill() {
    core::arch::naked_asm!(
        "
        .balign 4096
            csrwr   $t0, 0x8b
            csrrd   $t0, 0x1b
            lddir   $t0, $t0, 3
            lddir   $t0, $t0, 1
            ldpte   $t0, 0
            ldpte   $t0, 1
            tlbfill
            csrrd   $t0, 0x8b
            ertn
        ",
    );
}
