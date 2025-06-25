use core::arch::global_asm;
use crate::arch::TrapArch;

global_asm!(include_str!("trap.S"));

extern "C" {
    fn trap_vector_base();
}

pub struct LoongArch64Trap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoongArchException {
    Syscall = 11,           // System call
    PageFault = 1,          // Page fault (PIL)
    LoadPageFault = 2,      // Load page fault
    StorePageFault = 3,     // Store page fault
    InstructionPageFault = 4, // Instruction page fault
    IllegalInstruction = 12, // Illegal instruction
    Breakpoint = 14,        // Breakpoint
    LoadFault = 5,          // Load access fault
    StoreFault = 6,         // Store access fault
    InstructionFault = 7,   // Instruction access fault
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoongArchInterrupt {
    Timer = 11,             // Timer interrupt
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoongArchScause {
    bits: usize,
}

impl LoongArchScause {
    pub fn from_bits(bits: usize) -> Self {
        Self { bits }
    }
    
    pub fn is_interrupt(&self) -> bool {
        (self.bits & (1 << 63)) != 0
    }
    
    pub fn code(&self) -> usize {
        self.bits & 0x3FFF
    }
}

impl TrapArch for LoongArch64Trap {
    type Scause = LoongArchScause;
    type Exception = LoongArchException;
    type Interrupt = LoongArchInterrupt;
    
    fn init_trap() {
        Self::set_trap_vector();
        unsafe {
            core::arch::asm!(
                "li.d $t0, 0x1",
                "csrwr $t0, 0x2",  // CSR_EUEN
            );
        }
    }
    
    fn set_trap_vector() {
        unsafe {
            core::arch::asm!(
                "la.global $t0, {}",
                "csrwr $t0, 0xc",  // CSR_EENTRY
                sym trap_vector_base
            );
        }
    }
    
    fn enable_irqs() {
        unsafe {
            core::arch::asm!(
                "csrrd $t0, 0x4",    // CSR_ECFG
                "ori $t0, $t0, 0x800", // Set TI bit (bit 11)
                "csrwr $t0, 0x4",    // CSR_ECFG
            );
        }
    }
    
    fn disable_irqs() {
        unsafe {
            // 需要使用 andn 指令或者先加载掩码到寄存器
            core::arch::asm!(
                "csrrd $t0, 0x4",       // CSR_ECFG
                "li.d $t1, 0x800",      // Load mask
                "andn $t0, $t0, $t1",   // Clear TI bit using andn
                "csrwr $t0, 0x4",       // CSR_ECFG
            );
        }
    }
    
    fn enable_kernel_irqs() {
        unsafe {
            core::arch::asm!(
                "csrrd $t0, 0x0",    // CSR_CRMD
                "ori $t0, $t0, 0x4",  // Set IE bit (bit 2)
                "csrwr $t0, 0x0",    // CSR_CRMD
            );
        }
    }
    
    fn disable_kernel_irqs() {
        unsafe {
            // 使用 andn 指令清除位
            core::arch::asm!(
                "csrrd $t0, 0x0",       // CSR_CRMD
                "li.d $t1, 0x4",        // Load mask
                "andn $t0, $t0, $t1",   // Clear IE bit
                "csrwr $t0, 0x0",       // CSR_CRMD
            );
        }
    }
    
    fn wait_for_irqs() {
        unsafe {
            core::arch::asm!("idle 0");
        }
    }
    
    fn set_sum() {
        // 页表项的权限位控制用户内存访问
        unsafe {
            // 允许内核访问用户空间的标志
            core::arch::asm!(
                "csrrd $t0, 0x0",    // CSR_CRMD
                "ori $t0, $t0, 0x8",  // Set DATF bit (bit 3) - Direct Address Translation
                "csrwr $t0, 0x0",    // CSR_CRMD
            );
        }
    }
    
    fn read_scause() -> Self::Scause {
        let bits: usize;
        unsafe {
            core::arch::asm!(
                "csrrd {}, 0x5",     // CSR_ESTAT (Exception Status)
                out(reg) bits
            );
        }
        LoongArchScause::from_bits(bits)
    }
    
    fn read_stval() -> usize {
        let val: usize;
        unsafe {
            core::arch::asm!(
                "csrrd {}, 0x7",     // CSR_BADV (Bad Virtual Address)
                out(reg) val
            );
        }
        val
    }
    
    fn read_sepc() -> usize {
        let val: usize;
        unsafe {
            core::arch::asm!(
                "csrrd {}, 0x6",     // CSR_ERA (Exception Return Address)
                out(reg) val
            );
        }
        val
    }
    
    fn read_satp() -> usize {
        let val: usize;
        unsafe {
            core::arch::asm!(
                "csrrd {}, 0x1b",    // CSR_PGDH (Page Table Base High)
                out(reg) val
            );
        }
        val
    }
    
    fn is_syscall(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 11 // SYS exception
    }
    
    fn is_page_fault(cause: &Self::Scause) -> bool {
        if cause.is_interrupt() {
            return false;
        }
        match cause.code() {
            1 | 2 | 3 | 4 => true, // PIL, PIS, PIF, PME
            _ => false,
        }
    }
    
    fn is_timer_interrupt(cause: &Self::Scause) -> bool {
        cause.is_interrupt() && (cause.bits & 0x800) != 0 // Timer interrupt bit
    }
    
    fn is_illegal_instruction(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 12 // INE (Instruction Not Exist)
    }
    
    fn is_breakpoint(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 14 // BRK (Breakpoint)
    }
    
    fn is_store_fault(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 6 // ALE for store
    }
    
    fn is_load_fault(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 5 // ALE for load
    }
    
    fn is_instruction_fault(cause: &Self::Scause) -> bool {
        !cause.is_interrupt() && cause.code() == 7 // ADEF (Address Error for Fetch)
    }
    
    fn syscall_instruction_len() -> usize { 4 }
}