/// 获取当前运行的 CPU 核
pub fn hart_id() -> usize {
    use core::arch::asm;
    let hartid;
    unsafe {
        asm! {
            "mv {}, tp",
            out(reg) hartid
        };
    }
    hartid
}
