cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")] {
        mod riscv64;
        pub use riscv64::*;

    } else if #[cfg(target_arch = "loongarch64")] {
        mod loongarch64;
        pub use loongarch64::*;

    } else {
        compile_error!("Unsupported target architecture");
    }
}
