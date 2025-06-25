use cfg_if::cfg_if; 
cfg_if::cfg_if! {
    if #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))] {
        mod riscv64;
        
        pub use riscv64::*;

    }
    else if #[cfg(target_arch = "loongarch64")] {
        mod loongarch64;
        pub use loongarch64::*;
    } else {
        compile_error!("Unsupported architecture");
    }
}
