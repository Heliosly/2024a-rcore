//! Implementation of [`TrapContext`]
use core::arch::asm;

use loongArch64::register::prmd::{set_pplv, Prmd};

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
use riscv::register::{
    scause::{Exception, Interrupt, Trap},
    sstatus::{self, Sstatus, SPP},
};

use crate::{
    signal::{SigSet, SignalStack},
    task::{current_stack_top, current_task, TaskStatus},
    utils::page_round_up,
};

/// 用于表示内核处理是否处理完成，若处理完，则表示可以进入下一个阶段
#[repr(usize)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TrapStatus {
    #[default]
    Done = 0,
    Blocked = 1,
    Unknown,
}
#[cfg(target_arch="loongarch64")]
/// General registers of LoongArch64.
#[allow(missing_docs)]
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct GeneralRegisters {
    pub ra: usize,       // 1 返回地址
    pub tp: usize,       // 2 线程指针 (TLS)
    pub sp: usize,       // 3 栈指针
    pub a0: usize,       // 4 函数返回值 / 参数 0
    pub a1: usize,       // 5 参数 1
    pub a2: usize,       // 6 参数 2
    pub a3: usize,       // 7 参数 3
    pub a4: usize,       // 8 参数 4
    pub a5: usize,       // 9 参数 5
    pub a6: usize,       // 10 参数 6
    pub a7: usize,       // 11 系统调用号
    pub t0: usize,       // 12 临时寄存器
    pub t1: usize,       // 13 临时寄存器
    pub t2: usize,       // 14 临时寄存器
    pub t3: usize,       // 15 临时寄存器
    pub t4: usize,       // 16 临时寄存器
    pub t5: usize,       // 17 临时寄存器
    pub t6: usize,       // 18 临时寄存器
    pub t7: usize,       // 19 临时寄存器
    pub t8: usize,       // 20 临时寄存器
    pub gp: usize,       // r21
    pub fp: usize,       // 22 帧指针 / s0
    pub s0: usize,       // 23 被调用者保存
    pub s1: usize,       // 24 被调用者保存
    pub s2: usize,       // 25 被调用者保存
    pub s3: usize,       // 26 被调用者保存
    pub s4: usize,       // 27 被调用者保存
    pub s5: usize,       // 28 被调用者保存
    pub s6: usize,       // 29 被调用者保存
    pub s7: usize,       // 30 被调用者保存
    pub s8: usize,      // 31 被调用者保存
}



#[cfg(target_arch="riscv64")]
/// General registers of RISC-V.
#[allow(missing_docs)]
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct GeneralRegisters {
    pub ra: usize, //1
    pub sp: usize, //2
    pub gp: usize, // only valid for user traps3
    pub tp: usize, // only valid for user traps4
    pub t0: usize, //5
    pub t1: usize, //6
    pub t2: usize, //7
    pub s0: usize, //8
    pub s1: usize, //9
    pub a0: usize, //9
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize, //20
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize, //30
}
#[repr(C)]
#[derive(Debug, Clone, Copy)]
///trap context structure containing sstatus, sepc and registers
pub struct TrapContext {
    /// General-Purpose Register x0-30
    pub regs: GeneralRegisters,
    /// Supervisor Status Register
    pub sstatus: usize, //31
    /// Supervisor Exception Program Counter
    pub sepc: usize, //32
    /// Kernel stack pointer of the current application
    pub kernel_sp: usize, //33

    /// trap_loop
    pub kernel_ra: usize, //34
    /// s registers
    pub kernel_s: [usize; 12], // 35 - 46

    /// float regs 51-82, fcsr 83
    pub kernel_fp: usize, // 47
    /// kernel hart address
    pub kernel_tp: usize, // 48
    /// A copy of register a0, useful when we need to restart syscall
    pub origin_a0: usize, // 49
    /// float regs 51-82, fcsr 83
    pub fp: [usize; 32], // 50-81
    /// floating-point control and status register
    pub fcsr: usize, // 82
    /// Supervisor Cause Register
    pub scause: usize, //83
    /// Supervisor Trap Value
    pub stval: usize, //84
    /// 表示是否需要进行处理
    pub trap_status: TrapStatus, //85
}
fn read_sstatus() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        sstatus::read().bits()
    }
    #[cfg(target_arch = "loongarch64")]
    {
        loongArch64::register::prmd::read().raw()
    }
}
impl TrapContext {
    /// create a new TrapContext "不是全0"
    pub fn new() -> Self {
        Self {
            regs: Default::default(),
            sstatus: read_sstatus(),
            sepc: 0,
            kernel_sp: current_stack_top(),
            kernel_ra: 0,
            kernel_s: [0; 12],
            kernel_fp: 0,
            kernel_tp: 0,
            origin_a0: 0,
            fp: [0; 32],
            fcsr: 0,
            trap_status: Default::default(),
            scause: 0,
            stval: 0,
        }
    }
    pub fn set_tls(&mut self, arg: usize) {
        self.regs.tp = arg;
    }
    pub fn set_arg1(&mut self, arg: usize) {
        self.regs.a1 = arg;
    }
    pub fn set_arg2(&mut self, arg: usize) {
        self.regs.a2 = arg;
    }
    pub fn set_arg0(&mut self, arg: usize) {
        self.regs.a0 = arg;
    }
    pub fn get_pc(&self) -> usize {
        self.sepc
    }
    pub fn set_origin_a0(&mut self, arg: usize) {
        self.origin_a0 = arg;
    }
    ///set sepc
    pub fn set_pc(&mut self, pc: usize) {
        self.sepc = pc;
    }
    pub fn set_ra(&mut self, ra: usize) {
        self.regs.ra = ra;
    }
    /// put the user_sp(stack pointer) into x\[2\] field of TrapContext
    pub fn set_sp(&mut self, sp: usize) {
        self.regs.sp = sp;
    }
    ///get the user_sp
    pub fn get_sp(&self) -> usize {
        self.regs.sp
    }
    /// init the trap context of an application
    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        #[cfg(target_arch = "riscv64")]
        let mut sstatus = sstatus::read();
        #[cfg(target_arch = "loongarch64")]
        let mut prmd = {
            use loongArch64::register::prmd;
            prmd::read()
        };
        
        // set CPU privilege to User after trapping back
        #[cfg(target_arch = "riscv64")]
        {
            sstatus.set_spp(SPP::User);
        }

        #[cfg(target_arch = "loongarch64")]
        {
            // 设置特权级为用户态（Ring3）
            use loongArch64::register::prmd::set_pplv;
            set_pplv(loongArch64::register::CpuMode::Ring3);
        }

        #[cfg(target_arch = "riscv64")]
        let status_bits = sstatus.bits();
        #[cfg(target_arch = "loongarch64")]
        let status_bits = prmd.raw();

        trace!("read Status in app_init_trapcontext,{:#?}", status_bits);
        
        let mut cx = Self {
            regs: Default::default(),
            sstatus: status_bits,
            sepc: entry, // entry point of app
            kernel_sp: current_stack_top(), // kernel stack
            kernel_s: [0; 12],
            kernel_fp: 0,
            kernel_tp: 0,
            origin_a0: 0,
            fp: [0; 32],
            fcsr: 0,
            trap_status: TrapStatus::Done,
            scause: 0,
            stval: 0,
            kernel_ra: 114514,
        };
        cx.set_sp(sp); // app's user stack pointer
        cx // return initial Trap Context of app
    }

    ///get scause code
    pub fn get_scause_code(&self) -> usize {
        self.scause
    }
    pub fn init(&mut self, user_sp: usize, entry: usize) {
        self.set_sp(user_sp);
        self.sepc = entry;
    }
}
extern "C" {
    fn trap_return1(ctx: *mut TrapContext) -> !;
    fn user_return1(ctx: *mut TrapContext) -> !;
}

impl TrapContext {
    /// 内核态抢占恢复
    pub fn preempt_return(&self) -> ! {
        unsafe {
            let ctx_ptr = self as *const _ as *mut TrapContext;
            trap_return1(ctx_ptr);
        }
    }
    /// 获取 ret
    pub fn get_ret_code(&self) -> usize {
        self.regs.a0
    }
}
/// 用户态返回恢复
pub fn user_return(ctx: *mut TrapContext) -> ! {
    unsafe {
        user_return1(ctx);
    }
}
