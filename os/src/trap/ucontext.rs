use crate::signal::{SigSet, SignalStack};

use super::context::{GeneralRegisters, TrapContext};


#[repr(C, align(16))]
#[derive(Clone,Copy)]
pub struct MContext {
    pub pc: usize,
    pub regs: GeneralRegisters,
    fpstate: [usize; 66],
}

impl MContext {
    pub fn new(tf: &TrapContext) -> Self {
        Self {
            pc: tf.sepc,
            regs: tf.regs,
            fpstate: [0; 66],
        }
    }
    

    pub fn restore(&self, tf: &mut TrapContext) {
        tf.sepc = self.pc;
        tf.regs = self.regs;
    }
}

#[repr(C)]
#[derive(Clone,Copy)]
pub struct UContext {
    pub flags: usize,
    pub link: usize,
    pub stack: SignalStack,
    pub sigmask: SigSet,
    __unused: [u8; 1024 / 8 - size_of::<SigSet>()],
    pub mcontext: MContext,
}

impl UContext {
    pub fn new(tf: &TrapContext, sigmask: SigSet) -> Self {
        Self {
            flags: 0,
            link: 0,
            stack: SignalStack::default(),
            sigmask,
            __unused: [0; 1024 / 8 - size_of::<SigSet>()],
            mcontext: MContext::new(tf),
        }
    }
    pub fn init(pc:usize,sigmask:SigSet)->Self{
        Self{
            flags:0,
            link:0,
            stack:SignalStack::default(),
            sigmask,
            __unused: [0; 1024 / 8 - size_of::<SigSet>()],
            mcontext: MContext {
                pc,
                regs: GeneralRegisters::default(),
                fpstate: [0; 66],
            },
        }
    }
    pub fn get_pc(&self) -> usize {
        self.mcontext.pc
    }
    pub fn set_pc(&mut self, pc: usize) {
        self.mcontext.pc = pc;
    }
    // pub fn get_regs(&self) -> GeneralRegisters {
    //     self.mcontext.regs
    // }
    // pub fn set_regs(&mut self, regs: GeneralRegisters) {
    //     self.mcontext.regs = regs;
    // }
}
