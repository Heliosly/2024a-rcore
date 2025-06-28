use core::arch::global_asm;
use core::arch::asm;

global_asm!(include_str!("entry.asm"));
global_asm!(include_str!("signal.S"));

pub mod trap;
// pub mod context;
// pub mod mm;
// pub mod timer;
use crate::arch::ArchInit;
use crate::config::CLOCK_FREQ;
use crate::config::KERNEL_DIRECT_OFFSET;
use crate::sbi;
use crate::timer::UserTimeSpec;
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


use riscv::register;

pub use  riscv::register::scause::Trap;
pub struct Scause(register::scause::Scause);

impl Scause {
    pub fn cause(&self) ->Trap {
        self.0.cause()
    }
}

pub fn scause() -> Scause {
    Scause(register::scause::read())  
}

pub mod scause {
    use super::Scause;
    pub fn read() -> Scause {
        super::scause()
    }
}

pub mod sepc {
    pub fn read() -> usize {
        
            riscv::register::sepc::read()
        
    }
}

pub mod stval {
    use riscv::register::stval;

    pub fn read() -> usize {
       stval::read()
    }
}

pub fn enable_irqs() {
      
        unsafe { register::sie::set_stimer() };

}
pub fn disable_irqs() {
   
   unsafe { register::sie::clear_stimer() };

}



pub fn set_next_trigger(next: UserTimeSpec) {
     sbi::set_timer
        (
        next.tv_sec * CLOCK_FREQ + next.tv_nsec * CLOCK_FREQ / 1_000_000_000,
    );
}
