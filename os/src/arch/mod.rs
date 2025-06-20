pub trait ArchTrait {
    fn enable_irqs();
    fn disable_irqs();
    fn local_irq_save_and_disable() -> usize;
    fn local_irq_restore(flags: usize);
    fn activate_paging(token: usize);
    fn flush_tlb();
    fn enable_kernel_irqs();
    fn disable_kernel_irqs();
    fn wait_for_irqs();
    fn set_trap_entry(entry: usize);
}

#[cfg(target_arch = "riscv64")]
pub mod riscv64;
#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

#[cfg(target_arch = "loongarch64")]
pub mod loongarch64;
#[cfg(target_arch = "loongarch64")]
pub use loongarch64::*;

#[cfg(target_arch = "riscv64")]
pub type CurrentArch = riscv64::RiscV64Arch;

#[cfg(target_arch = "loongarch64")]
pub type CurrentArch = loongarch64::LoongArch64Arch;

pub use CurrentArch::*;