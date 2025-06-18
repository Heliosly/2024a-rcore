use core::mem;

use num_enum::{IntoPrimitive, TryFromPrimitive};

use crate::config::SS_DISABLE;

use super::{sigact::SignalDefaultAction, NSIG};
const PADDING_SIZE: usize = if cfg!(target_pointer_width = "64") { 4 } else { 0 };
/// ## 信号编号和元数据
///
/// `NSIG` 定义了支持的信号数量。
/// `SigInfo` 结构体包含了信号的编号和代码，对应于 libc 中的 `siginfo_t` 结构体。
/// `SignalStack` 结构体用于管理信号处理函数使用的栈。
/// `SigMaskHow` 枚举定义了信号掩码的操作方式，如阻塞、解除阻塞和设置掩码。
/// `Signal` 枚举定义了支持的信号类型，如 SIGHUP、SIGINT 等。
/// `SignalDefaultAction` 枚举定义了信号的默认行为，如终止进程、忽略信号等。
/// --- . 信号编号和元数据 ---
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigInfo {
    pub si_signo: u32,
    pub si_errno: u32,
    pub si_code: u32,
    pub _pad0: [u8; PADDING_SIZE],
    
    // unsupported fields
    pub _sifields: [u8; 112],
}
#[repr(C)]
pub struct SigInfoKill {
   pub pid: u32, 
   pub uid: u32, 
}
// 实现 Default，确保所有字段都被初始化为0
impl Default for SigInfo {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}


#[repr(C)]
#[derive(Clone, Debug, Copy)]
pub struct SignalStack {
    pub sp: usize,
    pub flags: u32,
    pub size: usize,
}
impl Default for SignalStack {
    fn default() -> Self {
        Self {
            sp: 0,
            flags: SS_DISABLE,
            size: 0,
        }
    }
}
impl SignalStack {
    pub fn new(sp: usize, size: usize) -> Self {
        SignalStack {
            sp,
            flags: SS_DISABLE,
            size,
        }
    }
}

impl SignalStack {
    pub fn disabled(&self) -> bool {
        self.flags == SS_DISABLE
    }
}

// 需要与 SigSet 的大小匹配
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(i32)] // libc::c_int 是 i32
#[allow(non_camel_case_types)]
pub enum SigMaskHow {
    SIG_BLOCK = 0,   // 0
    SIG_UNBLOCK = 1, // 1
    SIG_SETMASK = 2, // 2
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, IntoPrimitive, TryFromPrimitive)]
#[repr(usize)]
pub enum Signal {
    // POSIX.1-1990 Signals
    SIGHUP = 1,
    SIGINT = 2,
    SIGQUIT = 3,
    SIGILL = 4,
    SIGTRAP = 5,
    SIGABRT = 6, // Also SIGIOT
    SIGBUS = 7,
    SIGFPE = 8,
    SIGKILL = 9,
    SIGUSR1 = 10,
    SIGSEGV = 11,
    SIGUSR2 = 12,
    SIGPIPE = 13,
    SIGALRM = 14,
    SIGTERM = 15,
    SIGSTKFLT = 16, // Not in POSIX, but on Linux
    SIGCHLD = 17,
    SIGCONT = 18,
    SIGSTOP = 19, // Cannot be caught or ignored
    SIGTSTP = 20,
    SIGTTIN = 21,
    SIGTTOU = 22,
    SIGURG = 23,
    SIGXCPU = 24,
    SIGXFSZ = 25,
    SIGVTALRM = 26,
    SIGPROF = 27,
    SIGWINCH = 28,
    SIGIO = 29,  // Also SIGPOLL
    SIGPWR = 30, // Not in POSIX, but on Linux
    SIGSYS = 31, // Also SIGUNUSED
                 // Real-time signals could be added here SIGRTIN toSIGRTAX)
                 // For simplicity, we'll stick to standard signals for now.
                SIGRT1 = 32,
                SIGRT2 = 33,
                SIGRT3 = 34,
                SIGRT4 = 35,
                SIGRT5 = 36,
                SIGRT6 = 37,
                SIGRT7 = 38,
                SIGRT8 = 39,
                SIGRT9 = 40,
                SIGRT10 = 41,
                SIGRT11 = 42,
                SIGRT12 = 43,
                SIGRT13 = 44,
                SIGRT14 = 45,
                SIGRT15 = 46,
                SIGRT16 = 47,
                SIGRT17 = 48,
                SIGRT18 = 49,
                SIGRT19 = 50,
                SIGRT20 = 51,
                SIGRT21 = 52,
                SIGRT22 = 53,
                SIGRT23 = 54,
                SIGRT24 = 55,
                SIGRT25 = 56,
                SIGRT26 = 57,
                SIGRT27 = 58,
                SIGRT28 = 59,
                SIGRT29 = 60,
                SIGRT30 = 61,
                SIGRT31 = 62,
                SIGRT32 = 63,
SIGRTAX = 64,
}

impl Signal {
    pub fn from_usize(signum: usize) -> Option<Self> {
        if signum == 0 || signum >= NSIG {
            // 0 不是有效信号
            return None;
        }
        // SAFETY: 假设 Signal 枚举值与 usize 对应且在范围内
        // 这种转换在 repr(usize) 和值正确时是安全的。
        // 更安全的方式是使用 match 语句，但如果信号很多会很长。
        if signum <= 64 {
            // 假设我们只定义了到 31
            Some(unsafe { mem::transmute(signum) })
        } else {
            None // 未定义的信号
        }
    }

    pub fn default_action(&self) -> SignalDefaultAction {
        match self {
            Signal::SIGHUP
            | Signal::SIGINT
            | Signal::SIGQUIT
            | Signal::SIGILL
            | Signal::SIGTRAP
            | Signal::SIGABRT
            | Signal::SIGBUS
            | Signal::SIGFPE
            | Signal::SIGSEGV
            | Signal::SIGPIPE
            | Signal::SIGALRM
            | Signal::SIGTERM
            | Signal::SIGXCPU
            | Signal::SIGXFSZ
            | Signal::SIGVTALRM
            | Signal::SIGPROF
            | Signal::SIGSYS => SignalDefaultAction::Terminate,

            Signal::SIGKILL | Signal::SIGSTOP => SignalDefaultAction::ForceTerminateOrStop, // 特殊处理

            Signal::SIGCHLD | Signal::SIGURG | Signal::SIGWINCH | Signal::SIGCONT => {
                SignalDefaultAction::Ignore
            }

            Signal::SIGTSTP | Signal::SIGTTIN | Signal::SIGTTOU => SignalDefaultAction::Stop,
            _ => SignalDefaultAction::Terminate, // 其他未明确列出的默认为 Terminate
        }
    }
}