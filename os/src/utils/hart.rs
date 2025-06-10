/// 获取当前运行的 CPU 核
pub fn hart_id() -> usize {
    use core::arch::asm;
    let hartid;
    unsafe {
        #[cfg(target_arch = "riscv64")]
        asm! {
            "mv {}, tp",
            out(reg) hartid
        };
        #[cfg(target_arch = "loongarch64")]
        asm! {
            "move {}, $tp",
            out(reg) hartid
        };
    }
    hartid
}
