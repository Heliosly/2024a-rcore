//! SBI call wrappers and platform abstraction

#![allow(unused)]

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
mod riscv_sbi {
    use core::arch::asm;
    
    const SBI_SET_TIMER: usize = 0;
    const SBI_CONSOLE_PUTCHAR: usize = 1;
    const SBI_CONSOLE_GETCHAR: usize = 2;
    const SBI_SHUTDOWN: usize = 8;

    #[inline(always)]
    fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
        let mut ret;
        unsafe {
            asm!(
                "ecall",
                inlateout("x10") arg0 => ret,
                in("x11") arg1,
                in("x12") arg2,
                in("x16") 0,
                in("x17") which,
            );
        }
        ret
    }

    pub fn set_timer(timer: usize) {
        sbi_call(SBI_SET_TIMER, timer, 0, 0);
    }

    pub fn console_putchar(c: usize) {
        sbi_call(SBI_CONSOLE_PUTCHAR, c, 0, 0);
    }

    pub fn console_getchar() -> usize {
        sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0)
    }

    pub fn shutdown() -> ! {
        sbi_call(SBI_SHUTDOWN, 0, 0, 0);
        panic!("It should shutdown!");
    }
}

#[cfg(target_arch = "loongarch64")]
mod loongarch_platform {
    use core::arch::asm;

    use polyhal::VirtAddr;

    use crate::config::KERNEL_DIRECT_OFFSET;

    pub fn set_timer(timer: usize) {
        unsafe {
            // LoongArch64 定时器设置
            asm!(
                "csrwr {0}, 0x41",  // TCFG 寄存器
                in(reg) timer,
            );
        }
    }

    pub fn console_putchar(c: usize) {
        unsafe {
            // QEMU LoongArch64 UART 地址
            let uart_base = 0x1fe001e0usize;
            core::ptr::write_volatile(uart_base as *mut u8, c as u8);
        }
    }

    pub fn console_getchar() -> usize {
        unsafe {
            let uart_base = 0x1fe001e0usize;
            core::ptr::read_volatile(uart_base as *const u8) as usize
        }
    }

    #[inline]
    pub fn shutdown() -> ! {
        let ged_addr =VirtAddr::from (0x100E001C +KERNEL_DIRECT_OFFSET);
        log::info!("Shutting down...");
        unsafe { ged_addr.get_mut_ptr::<u8>().write_volatile(0x34) };
        unsafe { loongArch64::asm::idle() };
        log::warn!("It should shutdown!");
        unreachable!()
    }

    // QEMU LoongArch64 特定的关机实现
    unsafe fn qemu_loongarch64_shutdown() -> ! {
        // 方法1: 使用 QEMU 的调试退出接口
        // QEMU 在 0x100000 地址提供了调试退出功能
        let qemu_exit_addr = 0x100000usize as *mut u32;
        core::ptr::write_volatile(qemu_exit_addr, 0x5555); // QEMU 退出码
        
        // 方法2: 如果上面不工作，尝试 ACPI 关机
        // PM1a_CNT 寄存器
        let pm1a_cnt = 0x404usize as *mut u16;
        let shutdown_value = 0x2000u16 | 0x0400u16;
        core::ptr::write_volatile(pm1a_cnt, shutdown_value);
        
        // 方法3: 最后的回退方案 - 正确的 idle 指令
        // 这个循环确保函数永远不会返回
        loop {
            // LoongArch64 idle 指令需要一个立即数参数
            asm!("idle 0", options(nostack, preserves_flags));  // 参数 0 表示进入最低功耗状态
        }
    }
}


#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub use riscv_sbi::*;

#[cfg(target_arch = "loongarch64")]
pub use loongarch_platform::*;