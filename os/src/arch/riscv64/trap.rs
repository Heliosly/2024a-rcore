use core::arch::global_asm;
use riscv::register::{
    mstatus::FS,
    mtvec::TrapMode,
    sie, sstatus, stvec,
    scause::{self, Exception, Interrupt, Trap, Scause},
    stval, sepc, satp,
};
use super::super::TrapArch;

global_asm!(include_str!("trap.S"));

extern "C" {
    fn trap_vector_base();
}

pub struct RiscV64Trap;

impl TrapArch for RiscV64Trap {
    type Scause = Scause;
    type Exception = Exception;
    type Interrupt = Interrupt;
    
    fn init_trap() {
        Self::set_trap_vector();
        unsafe {
            sstatus::set_fs(FS::Clean);
        }
    }
    
    fn set_trap_vector() {
        unsafe {
            stvec::write(trap_vector_base as usize, TrapMode::Direct);
        }
    }
    
    fn enable_irqs() {
        unsafe {
            sie::set_stimer();
        }
    }
    
    fn disable_irqs() {
        unsafe {
            sie::clear_stimer();
        }
    }
    
    fn enable_kernel_irqs() {
        unsafe { 
            sstatus::set_sie() 
        }
    }
    
    fn disable_kernel_irqs() {
        unsafe { 
            sstatus::clear_sie() 
        }
    }
    
    fn wait_for_irqs() {
        unsafe {
            riscv::asm::wfi()
        }
    }
    
    fn set_sum() {
        unsafe { 
            sstatus::set_sum() 
        };
    }
    
    fn read_scause() -> Self::Scause {
        scause::read()
    }
    
    fn read_stval() -> usize {
        stval::read()
    }
    
    fn read_sepc() -> usize {
        sepc::read()
    }
    
    fn read_satp() -> usize {
        satp::read().bits()
    }
    
    fn is_syscall(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::UserEnvCall))
    }
    
    fn is_page_fault(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), 
            Trap::Exception(Exception::StorePageFault) |
            Trap::Exception(Exception::LoadPageFault) |
            Trap::Exception(Exception::InstructionPageFault)
        )
    }
    
    fn is_timer_interrupt(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Interrupt(Interrupt::SupervisorTimer))
    }
    
    fn is_illegal_instruction(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::IllegalInstruction))
    }
    
    fn is_breakpoint(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::Breakpoint))
    }
    
    fn is_store_fault(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::StoreFault))
    }
    
    fn is_load_fault(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::LoadFault))
    }
    
    fn is_instruction_fault(cause: &Self::Scause) -> bool {
        matches!(cause.cause(), Trap::Exception(Exception::InstructionFault))
    }
    fn syscall_instruction_len() -> usize { 4 }
}
