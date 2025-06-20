use super::ArchTrait;

pub mod context;
pub mod trap;

pub struct LoongArch64Arch;

impl ArchTrait for LoongArch64Arch {
    #[inline]
    /// 启用时钟中断
    fn enable_irqs() {
        unsafe {
            // ECFG.LIE[11] = 1
            core::arch::asm!("csrxchg {}, {}, 0x4", 
                in(reg) 1 << 11,
                in(reg) 1 << 11
            );
        }
    }
    
    #[inline]
    /// 禁用时钟中断
    fn disable_irqs() {
        unsafe {
            // ECFG.LIE[11] = 0
            core::arch::asm!("csrxchg {}, {}, 0x4", 
                in(reg) 0,
                in(reg) 1 << 11
            );
        }
    }
    
    #[inline]
    fn local_irq_save_and_disable() -> usize {
        let flags: usize;
        unsafe {
            // 读取 PRMD
            core::arch::asm!("csrrd {}, 0x1", out(reg) flags);
            // PRMD.PIE = 0
            core::arch::asm!("csrxchg {}, {}, 0x1", 
                in(reg) 0,
                in(reg) 1 << 2
            );
        }
        flags
    }
    
    #[inline]
    fn local_irq_restore(flags: usize) {
        unsafe {
            // 恢复 PRMD 寄存器的中断状态
            let pie_bit = (flags >> 2) & 1;
            core::arch::asm!("csrxchg {}, {}, 0x1", 
                in(reg) pie_bit << 2,  // 恢复 PIE
                in(reg) 1 << 2
            );
        }
    }
    
    #[inline]
    fn activate_paging(token: usize) {
        unsafe {
            core::arch::asm!("csrwr {}, 0x19", in(reg) token);     // PGDL
            core::arch::asm!("csrwr {}, 0x1a", in(reg) token);     // PGDH
            Self::flush_tlb();
        }
    }
    
    #[inline]
    fn flush_tlb() {
        unsafe {
            core::arch::asm!("tlbflush");
        }
    }
    
    #[inline]
    fn enable_kernel_irqs() {
        unsafe {
            // PRMD.PIE = 1)
            core::arch::asm!("csrxchg {}, {}, 0x1", 
                in(reg) 1 << 2,  // PIE = 1
                in(reg) 1 << 2   // mask
            );
        }
    }
    
    #[inline]
    fn disable_kernel_irqs() {
        unsafe {
            // 禁用全局中断 (PRMD.PIE = 0)
            core::arch::asm!("csrxchg {}, {}, 0x1", 
                in(reg) 0,       // PIE = 0
                in(reg) 1 << 2   // mask
            );
        }
    }
    
    #[inline]
    fn wait_for_irqs() {
        unsafe {
            // LoongArch 的等待指令
            core::arch::asm!("idle 0");
        }
    }
    
    #[inline]
    fn set_trap_entry(entry: usize) {
        unsafe {
            // LoongArch 设置异常入口地址 (EENTRY)
            core::arch::asm!("csrwr {}, 0xc", in(reg) entry);
        }
    }
}

// 重新导出 LoongArch 特定的上下文类型
pub use context::*;