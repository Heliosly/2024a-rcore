//! LoongArch64 TrapContext implementation

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GeneralRegisters {
    pub ra: usize,  // x1
    pub tp: usize,  // x2  
    pub sp: usize,  // x3
    pub a0: usize,  // x4
    pub a1: usize,  // x5
    pub a2: usize,  // x6
    pub a3: usize,  // x7
    pub a4: usize,  // x8
    pub a5: usize,  // x9
    pub a6: usize,  // x10
    pub a7: usize,  // x11
    pub t0: usize,  // x12
    pub t1: usize,  // x13
    pub t2: usize,  // x14
    pub t3: usize,  // x15
    pub t4: usize,  // x16
    pub t5: usize,  // x17
    pub t6: usize,  // x18
    pub t7: usize,  // x19
    pub t8: usize,  // x20
    pub reserved: usize, // x21
    pub fp: usize,  // x22
    pub s0: usize,  // x23
    pub s1: usize,  // x24
    pub s2: usize,  // x25
    pub s3: usize,  // x26
    pub s4: usize,  // x27
    pub s5: usize,  // x28
    pub s6: usize,  // x29
    pub s7: usize,  // x30
    pub s8: usize,  // x31
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct TrapContext {
    pub regs: GeneralRegisters,
    pub era: usize,     // 异常返回地址，类似 sepc
    pub prmd: usize,    // 机器状态，类似 sstatus
    pub kernel_sp: usize,
    pub estat: usize,   // 异常状态寄存器
    pub badv: usize,    // 出错虚拟地址
    pub trap_status: usize,
}

impl TrapContext {
    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        let mut prmd = 0usize;
        prmd |= 1 << 2; // PLV = 3 (用户态)
        prmd |= 1 << 2; // PIE = 1 (开中断)
        
        let mut cx = Self {
            regs: GeneralRegisters::default(),
            era: entry,
            prmd,
            kernel_sp: 0,
            estat: 0,
            badv: 0,
            trap_status: 0,
        };
        cx.regs.sp = sp;
        cx
    }
    
    pub fn set_sp(&mut self, sp: usize) {
        self.regs.sp = sp;
    }
}

extern "C" {
    fn loongarch_trap_return();
    fn loongarch_user_return();
}

pub fn trap_return() -> ! {
    unsafe {
        loongarch_trap_return();
    }
    unreachable!();
}

pub fn user_return(ctx: &TrapContext) -> ! {
    unsafe {
        core::arch::asm!(
            "move $a0, {}",
            "b loongarch_user_return",
            in(reg) ctx as *const _ as usize,
            options(noreturn)
        );
    }
}