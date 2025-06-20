use super::ArchTrait;
use riscv::register::{sstatus, satp};

pub mod context;
pub mod trap;

pub struct RiscV64Arch;

impl ArchTrait for RiscV64Arch {
    #[inline]
    fn enable_irqs() {
        unsafe { sstatus::set_sie(); }
    }
    
    #[inline]
    fn disable_irqs() {
        unsafe { sstatus::clear_sie(); }
    }
    
    #[inline]
    fn local_irq_save_and_disable() -> usize {
        crate::kernel_guard::local_irq_save_and_disable()
    }
    
    #[inline]
    fn local_irq_restore(flags: usize) {
        crate::kernel_guard::local_irq_restore(flags)
    }
    
    #[inline]
    fn activate_paging(token: usize) {
        unsafe {
            satp::write(token);
            core::arch::asm!("sfence.vma");
        }
    }
    
    #[inline]
    fn flush_tlb() {
        unsafe {
            core::arch::asm!("sfence.vma");
        }
    }
    
    #[inline]
    fn enable_kernel_irqs() {
        unsafe { sstatus::set_sie(); }
    }
    
    #[inline]
    fn disable_kernel_irqs() {
        unsafe { sstatus::clear_sie(); }
    }
    
    #[inline]
    fn wait_for_irqs() {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
    
    #[inline]
    fn set_trap_entry(entry: usize) {
        unsafe {
            core::arch::asm!("csrw stvec, {}", in(reg) entry);
        }
    }
}

pub use context::*;
