//! Implementation of [`TrapContext`]
use core::arch::asm;

use riscv::register::{scause::{Exception, Interrupt, Trap}, sstatus::{self, Sstatus, SPP}};

use crate::{signal::{SigSet, SignalStack}, task::{current_stack_top, current_task, TaskStatus}, utils::{ page_round_up}};


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

/// General registers of RISC-V.
#[allow(missing_docs)]
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct GeneralRegisters {
    pub ra: usize,
    pub sp: usize,
    pub gp: usize, // only valid for user traps
    pub tp: usize, // only valid for user traps
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
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
    pub s5: usize,//20
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
#[derive(Debug,Clone,Copy)]
///trap context structure containing sstatus, sepc and registers
pub struct TrapContext {
    /// General-Purpose Register x0-30
    pub regs: GeneralRegisters,
    /// Supervisor Status Register
    pub sstatus: Sstatus,//31
    /// Supervisor Exception Program Counter
    pub sepc: usize,//32
    /// Kernel stack pointer of the current application
    pub kernel_sp: usize,//33

    /// trap_loop 
    pub kernel_ra :usize,//34
    /// s registers
    pub kernel_s: [usize; 12], // 35 - 46
   
    /// float regs 51-82, fcsr 83
    pub kernel_fp: usize,      // 47
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
        pub stval: usize,//84
        /// 表示是否需要进行处理
        pub trap_status: TrapStatus,//85

}

impl TrapContext {
    /// create a new TrapContext "不是全0"
    pub fn new()->Self{
        Self {
            regs: Default::default(),
            sstatus: sstatus::read(), 
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
    pub fn set_tls(&mut self,arg:usize){
        self.regs.tp=arg;
    }
    pub fn set_arg1(&mut self,arg:usize){
        self.regs.a1=arg;
    }
    pub fn set_arg2(&mut self,arg:usize){
        self.regs.a2=arg;
    }
    pub fn set_arg0(&mut self,arg:usize){
        self.regs.a0=arg;
    }
    pub fn get_pc(&self)->usize{
        self.sepc
    }
    ///set sepc
    pub fn set_pc(&mut self, pc: usize) {
        self.sepc = pc;
    }
    pub fn set_ra(&mut self, ra: usize){
        self.regs.ra = ra;
    }
    /// put the user_sp(stack pointer) into x\[2\] field of TrapContext
    pub fn set_sp(&mut self, sp: usize) {
        self.regs.sp = sp;
    }
    ///get the user_sp 
    pub fn get_sp(&self)->usize{
        self.regs.sp
    }
    /// init the trap context of an application
    pub fn app_init_context(
        entry: usize,
        sp: usize,
    ) -> Self {
        let mut sstatus = sstatus::read();
        // set CPU privilege to User after trapping back
        sstatus.set_spp(SPP::User);

        trace!("read Sstaus in app_init_trapcontext,{:#?}",sstatus.bits());
        let mut cx = Self {
            regs: Default::default(),
            sstatus,
            sepc: entry,  // entry point of app
          

            kernel_sp:current_stack_top(),    // kernel stack
            kernel_s: [0; 12],
            kernel_fp: 0,
            kernel_tp: 0,
            origin_a0: 0,
            fp: [0; 32],
            fcsr: 0,
            trap_status:TrapStatus::Done,
            scause:0,
            stval:0,
            kernel_ra: 114514,
        
        };
        cx.set_sp(sp); // app's user stack pointer
        cx // return initial Trap Context of app
    }
    ///get scause type
    pub fn get_scause_type(&self) -> Trap {
        let is_interrupt = self.scause & (1 << (usize::BITS as usize - 1)) != 0;
        let code = self.scause & !(1 << (usize::BITS as usize - 1));
        if is_interrupt {
            Trap::Interrupt(Interrupt::from(code))
        } else {
            Trap::Exception(Exception::from(code))
        }
    }
    ///get scause code
    pub fn get_scause_code(&self) -> usize {
        self.scause
    }
    pub fn init(&mut self,user_sp:usize,entry: usize){
           self.set_sp(user_sp);
           self.sepc = entry;
    }
    


}
extern "C" {
    fn trap_return1();
    fn user_return1();
}

impl TrapContext{
    /// 内核态抢占恢复
    pub fn preempt_return(&self) -> ! {
        unsafe {
            let ctx_ptr = self as *const _ as usize;
            asm!(
                "mv a0, {0}",
                "jalr zero, {1}, 0", // 无返回跳转
                in(reg) ctx_ptr,
                in(reg) trap_return1 as usize,
                options(noreturn)
            );
        }
    }
 /// 获取 ret
 pub fn get_ret_code(&self) -> usize {
    self.regs.a0
}

    
}
/// 用户态返回恢复
    pub fn user_return(ctx:*mut TrapContext) -> ! {
        unsafe {
            let ctx_ptr = ctx as *const _ as usize;
            asm!(
                "mv a0, {0}",
                "jalr zero, {1}, 0",
                in(reg) ctx_ptr,
                in(reg) user_return1 as usize,
                options(noreturn)
            );
        }
    }