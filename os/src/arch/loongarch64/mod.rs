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
use crate::timer::current_time;
use crate::timer::get_usertime;
use crate::timer::TimeVal;
use crate::timer::UserTimeSpec;

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
use loongArch64::register;
use loongArch64::register::estat;
use loongArch64::register::tcfg; 

pub struct Scause(estat::Estat);
pub use  loongArch64::register::estat::Trap;
impl Scause {
    pub fn cause(&self) ->estat::Trap {
        self.0.cause()
    }
}

pub fn scause() -> Scause {
    Scause(estat::read())  
}

pub mod scause {
    use super::Scause;
    pub fn read() -> Scause {
        super::scause()
    }
}

pub mod sepc {
    pub fn read() -> usize {
        
            loongArch64::register::era::read().raw()
        
    }
}

pub mod stval {
    use loongArch64::register::badv;

    pub fn read() -> usize {
       badv::read().raw()
    }
}

pub fn enable_irqs() {
      loongArch64::register::crmd::set_ie(true);
}
pub fn disable_irqs() {
   
      loongArch64::register::crmd::set_ie(false);

}

pub fn set_next_trigger(next:UserTimeSpec) {
    let curr = get_usertime();
    if next < curr {
        return;
    }
    let interval = next - curr;
    tcfg::set_init_val(
        (interval.tv_sec * crate::config::CLOCK_FREQ
            + interval.tv_nsec  * crate::config::CLOCK_FREQ / 1_000_000_000) as _,
    );
    tcfg::set_en(true);
}