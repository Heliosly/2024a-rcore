use std::{env, io::Result};


fn main() -> Result<()> {
    gen_linker_script()
}

fn gen_linker_script() -> Result<()> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("can't find target");
    let fname = format!("linker_{}.lds", arch);
    let (output_arch, kernel_base) = if arch.contains("riscv64") {
        ("riscv", "0xffffffc080200000")
    } else if arch.contains("loongarch64") {
        ("loongarch64", "0x9000000080200000")
    } else {
        (arch.as_str(), "0")
    };
    
    let ld_content = std::fs::read_to_string("linker.lds")?;
    let ld_content = ld_content.replace("%ARCH%", output_arch);
    let ld_content = ld_content.replace("%KERNEL_BASE%", kernel_base);

    // 将生成的链接脚本写入 src 目录
    let output_path = format!("src/{}", fname);
    std::fs::write(&output_path, ld_content)?;
    
    // 告诉 rustc 使用这个链接脚本
    // 链接参数
    println!("cargo:rustc-link-arg=-T{}", output_path);
    println!("cargo:rustc-link-arg=-nostdlib");

    
    println!("cargo:rerun-if-env-changed=CARGO_CFG_KERNEL_BASE");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=linker.lds");
    Ok(())
}