use crate::arch::loongarch64::context::{LoongArchTrap, csr};
use crate::trap::TrapContext;

global_asm!(include_str!("trap.S"));

extern "C" {
    fn loongarch_trap_vector();
    fn loongarch_trap_return();
    fn loongarch_user_return();
}

pub fn init_trap() {
    unsafe {
        core::arch::asm!("csrwr {}, 0xc", in(reg) loongarch_trap_vector as usize);
    }
}

pub fn user_return(trap_cx: *mut TrapContext) {
    unsafe {
        loongarch_user_return();
    }
}

pub fn trap_return(trap_cx: *mut TrapContext) {
    unsafe {
        loongarch_trap_return();
    }
}