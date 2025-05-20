use crate::trap::TrapContext;

use super::{signal::{Signal, SignalStack}, NSIG};

/// ## 信号处理动作
///
/// `SigAction` 结构体定义了信号处理动作，包括信号处理函数、标志和掩码。
/// `SigActionFlags` 结构体定义了信号处理动作的标志，如 SA_NOCLDSTOP、SA_SIGINFO 等。
/// `SigSet` 结构体表示一组信号，通常使用位掩码实现。
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalDefaultAction {
    Terminate,            // 终止进程
    Ignore,               // 忽略信号
    CoreDump,             // 终止进程并转储核心 (简化为 Terminate)
    Stop,                 // 停止进程
    Continue,             // 继续已停止的进程
    ForceTerminateOrStop, // 如 SIGKILL, SIGSTOP, 不能被捕获或忽略 (内核特殊处理)
}

// mirrors struct sigaction from C, but with Rust types
#[derive(Clone, Copy, Debug)]
#[repr(C)] // 如果需要与 C ABI 兼容或从用户空间直接传递
pub struct SigAction {
    /// Signal handler (function pointer or SIG_DFL/SIG_IGN)
    /// Using usize to store function pointer or special values.
    /// 0 for SIG_DFL, 1 for SIG_IGN. Other values are handler addresses.
    pub handler: usize, // sa_handler or sa_sigaction (if SA_SIGINFO is set)
    pub flags: SigActionFlags, // sa_flags
    pub restorer: usize,       // sa_restorer (optional, for sigreturn trampoline)
    // ^ 通常由内核或libc设置，用户空间不应直接修改
    pub mask: SigSet, // sa_mask (signals to block during handler execution)
}

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

bitflags! {
    #[derive(Default)] // Default is all flags off
    pub struct SigActionFlags: u32 {
        const SA_NOCLDSTOP = 1;       /* Don't send SIGCHLD when children stop. */
        const SA_NOCLDWAIT = 2;       /* Don't create zombies. */
        const SA_SIGINFO   = 4;       /* Send signal with siginfo_t argument. */
        const SA_RESTORER  = 0x04000000; /* Has a restorer. */ // Linux specific value
        const SA_ONSTACK   = 0x08000000; /* Use signal stack. */
        const SA_RESTART   = 0x10000000; /* Restart syscalls. */
        const SA_NODEFER   = 0x40000000; /* Don't block signal in handler. */
        const SA_RESETHAND = 0x80000000; /* Reset handler to SIG_DFL on entry. */
        // Add more flags as needed
    }
}

impl Default for SigAction {
    fn default() -> Self {
        Self {
            handler: SIG_DFL,
            flags: SigActionFlags::empty(),
            restorer: 0, // Usually set by C library or kernel
            mask: SigSet::empty(),
        }
    }
}
impl SigAction {
   pub  fn get_restorer(&self) -> Option<usize> {
        match self.restorer {
            0 => None,
            f => Some(f),
        }
    }
}
// Represents a set of signals, typically using a bitmask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)] // So it's just like u64/usize array
pub struct SigSet {
    // Assuming NSIG <= 64. If more, use an array like [u64; NSIG / 64].
    bits: u64,
}

impl SigSet {
    pub fn empty() -> Self {
        Self { bits: 0 }
    }
    pub fn full() -> Self {
        Self { bits: u64::MAX }
    } // Blocks all possible signals (up to 64)

    pub fn add(&mut self, sig: Signal) {
        self.bits |= 1u64 << (sig as usize - 1); // Signals are 1-indexed
    }

    pub fn remove(&mut self, sig: Signal) {
        self.bits &= !(1u64 << (sig as usize - 1));
    }

    pub fn contains(&self, sig: Signal) -> bool {
        (self.bits >> (sig as usize - 1)) & 1 != 0
    }

    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }

    // Union, intersection, etc. can be added
    pub fn union_with(&mut self, other: &SigSet) {
        self.bits |= other.bits;
    }
    pub fn intersect_with(&mut self, other: &SigSet) {
        self.bits &= other.bits;
    }
    // ... and so on
}

#[derive(Clone, Debug)]
pub struct TaskSignalState {
    pub sig_info: bool,
    pub sigpending: SigSet, // 挂起的信号 (per-task)
    pub sigmask: SigSet,    // 当前阻塞的信号 (per-task)
    // pub shared_sigpending: Option<Arc<Mutex<SigSet>>>, // 如果有线程组共享的挂起信号
    pub last_context: Option<TrapContext>, // ss_t for sigaltstack
    /// Alternative signal stack
    pub alternate_stack: SignalStack,
}

impl TaskSignalState {
   pub fn init(sigmask:SigSet) -> Self {
        Self {
           sig_info:false,
           sigpending:SigSet::empty(),
           sigmask,
           last_context:None,
           alternate_stack:SignalStack::default(),
        }
    }
}
impl Default for TaskSignalState {
    fn default() -> Self {
        Self {
            sig_info: false,
            sigpending: SigSet::empty(),
            sigmask: SigSet::empty(),
            last_context: None,
            alternate_stack: SignalStack::default(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProcessSignalSharedState {
    pub sigactions: [SigAction; NSIG], // 进程共享的信号处理动作
    pub shared_sigpending: SigSet,     // 进程级别的挂起信号
                                       //线程不安全
}

impl Default for ProcessSignalSharedState {
    fn default() -> Self {
        let mut actions = [SigAction::default(); NSIG];
        // 初始化默认动作
        for i in 1..NSIG {
            // 信号从1开始
            if let Some(sig) = Signal::from_usize(i) {
                // SIGKILL and SIGSTOP cannot be changed from DFL.
                // Kernel usually enforces this in sys_sigaction.
                // Here, we just initialize.
                // Default is already SIG_DFL (0).
            }
        }
        Self {
            sigactions: actions,
            shared_sigpending: SigSet::empty(),
        }
    }
}
impl ProcessSignalSharedState{
    pub fn clone_from_another(state:&Self)->Self{
         Self {
             sigactions: state.sigactions,
             shared_sigpending:SigSet::empty(),
             }
    }
}