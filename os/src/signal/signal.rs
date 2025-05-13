use crate::task::exit_current_and_run_next;

/// 仿照Linux signal实现
pub const SIGHUP: usize = 1; /* Hangup.  */
pub const SIGINT: usize = 2; /* Interactive attention signal.  */
pub const SIGQUIT: usize = 3; /* Quit.  */
pub const SIGILL: usize = 4; /* Illegal instruction.  */
pub const SIGTRAP: usize = 5; /* Trace/breakpoint trap.  */
// aka SIGIOT
pub const SIGABRT: usize = 6; /* Abnormal termination.  */
pub const SIGBUS: usize = 7; /* Bus error.  */
pub const SIGFPE: usize = 8; /* Erroneous arithmetic operation.  */
pub const SIGKILL: usize = 9; /* Killed.  */
pub const SIGUSR1: usize = 10;
pub const SIGSEGV: usize = 11; /* Invalid access to storage.  */
pub const SIGUSR2: usize = 12;
pub const SIGPIPE: usize = 13; /* Broken pipe.  */
pub const SIGALRM: usize = 14; /* Alarm clock.  */
pub const SIGTERM: usize = 15; /* Termination request.  */
pub const SIGSTKFLT: usize = 16; /* Stack fault (obsolete).  */
// aka SIGCLD
pub const SIGCHLD: usize = 17; /* Child terminated or stopped.  */
pub const SIGCONT: usize = 18; /* Continue.  */
pub const SIGSTOP: usize = 19; /* Stop, unblockable.  */
pub const SIGTSTP: usize = 20; /* Keyboard stop.  */
pub const SIGTTIN: usize = 21; /* Background read from control terminal.  */
pub const SIGTTOU: usize = 22; /* Background write to control terminal.  */
pub const SIGURG: usize = 23; /* Urgent data is available at a socket.  */
pub const SIGXCPU: usize = 24; /* CPU time limit exceeded.  */
pub const SIGXFSZ: usize = 25; /* File size limit exceeded.  */
pub const SIGVTALRM: usize = 26; /* Virtual timer expired.  */
pub const SIGPROF: usize = 27; /* Profiling timer expired.  */
pub const SIGWINCH: usize = 28; /* Window size change (4.3 BSD, Sun).  */
// aka SIGPOLL
pub const SIGIO: usize = 29; /* Pollable event occurred (System V).  */
pub const SIGPWR: usize = 30; /* Power failure imminent.  */
pub const SIGSYS: usize = 31; /* Bad system call.  */
pub const SIGRTMIN: usize = 32;
// User Custom
pub const SIGRT_1: usize = SIGRTMIN + 1;

bitflags! {
    pub struct SigSet: usize {
        const SIGHUP    = 1 << (SIGHUP -1);
        const SIGINT    = 1 << (SIGINT - 1);
        const SIGQUIT   = 1 << (SIGQUIT - 1);
        const SIGILL    = 1 << (SIGILL - 1);
        const SIGTRAP   = 1 << (SIGTRAP - 1);
        const SIGABRT   = 1 << (SIGABRT - 1);
        const SIGBUS    = 1 << (SIGBUS - 1);
        const SIGFPE    = 1 << (SIGFPE - 1);
        const SIGKILL   = 1 << (SIGKILL - 1);
        const SIGUSR1   = 1 << (SIGUSR1 - 1);
        const SIGSEGV   = 1 << (SIGSEGV - 1);
        const SIGUSR2   = 1 << (SIGUSR2 - 1);
        const SIGPIPE   = 1 << (SIGPIPE - 1);
        const SIGALRM   = 1 << (SIGALRM - 1);
        const SIGTERM   = 1 << (SIGTERM - 1);
        const SIGSTKFLT = 1 << (SIGSTKFLT- 1);
        const SIGCHLD   = 1 << (SIGCHLD - 1);
        const SIGCONT   = 1 << (SIGCONT - 1);
        const SIGSTOP   = 1 << (SIGSTOP - 1);
        const SIGTSTP   = 1 << (SIGTSTP - 1);
        const SIGTTIN   = 1 << (SIGTTIN - 1);
        const SIGTTOU   = 1 << (SIGTTOU - 1);
        const SIGURG    = 1 << (SIGURG - 1);
        const SIGXCPU   = 1 << (SIGXCPU - 1);
        const SIGXFSZ   = 1 << (SIGXFSZ - 1);
        const SIGVTALRM = 1 << (SIGVTALRM - 1);
        const SIGPROF   = 1 << (SIGPROF - 1);
        const SIGWINCH  = 1 << (SIGWINCH - 1);
        const SIGIO     = 1 << (SIGIO - 1);
        const SIGPWR    = 1 << (SIGPWR - 1);
        const SIGSYS    = 1 << (SIGSYS - 1);
        const SIGRTMIN  = 1 << (SIGRTMIN- 1);
        const SIGRT_1   = 1 << (SIGRT_1 - 1);
    }
}

impl SigSet {
    pub fn default_op(&self) -> SigOp {
        let terminate_signals = SigSet::SIGHUP
            | SigSet::SIGINT
            | SigSet::SIGKILL
            | SigSet::SIGUSR1
            | SigSet::SIGUSR2
            | SigSet::SIGPIPE
            | SigSet::SIGALRM
            | SigSet::SIGTERM
            | SigSet::SIGSTKFLT
            | SigSet::SIGVTALRM
            | SigSet::SIGPROF
            | SigSet::SIGIO
            | SigSet::SIGPWR;
        let dump_signals = SigSet::SIGQUIT
            | SigSet::SIGILL
            | SigSet::SIGTRAP
            | SigSet::SIGABRT
            | SigSet::SIGBUS
            | SigSet::SIGFPE
            | SigSet::SIGSEGV
            | SigSet::SIGXCPU
            | SigSet::SIGXFSZ
            | SigSet::SIGSYS;
        let ignore_signals = SigSet::SIGCHLD | SigSet::SIGURG | SigSet::SIGWINCH;
        let stop_signals = SigSet::SIGSTOP | SigSet::SIGTSTP | SigSet::SIGTTIN | SigSet::SIGTTOU;
        let continue_signals = SigSet::SIGCONT;
        if terminate_signals.contains(*self) {
            SigOp::Terminate
        } else if dump_signals.contains(*self) {
            SigOp::CoreDump
        } else if ignore_signals.contains(*self) || self.bits == 0 {
            SigOp::Ignore
        } else if stop_signals.contains(*self) {
            SigOp::Stop
        } else if continue_signals.contains(*self) {
            SigOp::Continue
        } else {
            // println!("[kernel] signal {:?}: undefined default operation", self);
            SigOp::Terminate
        }
    }

    pub fn from_sig(signo: usize) -> Self {
        SigSet::from_bits(1 << (signo - 1)).unwrap()
    }
    pub fn peek_front(&self) -> Option<usize> {
        if self.is_empty() {
            None
        } else {
            // SigSet::from_bits(1 << (self.bits().trailing_zeros() as usize))
            Some(self.bits().trailing_zeros() as usize + 1)
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SigAction {
    pub sa_handler: usize,
    pub sa_flags: SigActionFlags,
    pub sa_restore: usize,
    pub sa_mask: SigSet,
}

impl SigAction {
    pub fn new(signo: usize) -> Self {
        let handler: usize = if signo == 0 {
            1
        } else {
            match SigSet::from_sig(signo).default_op() {
                SigOp::Continue | SigOp::Ignore => 1,
                SigOp::Stop => 1, // TODO(ZMY): 添加Stop状态和相关函数
                SigOp::Terminate | SigOp::CoreDump => exit_current_and_run_next as usize,
            }
        };
        Self {
            sa_handler: handler,
            sa_flags: SigActionFlags::empty(),
            sa_restore: 0,
            sa_mask: SigSet::empty(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct KSigAction {
    pub act: SigAction,
    pub customed: bool,
}

impl KSigAction {
    pub fn new(signo: usize, customed: bool) -> Self {
        Self {
            act: SigAction::new(signo),
            customed,
        }
    }
    pub fn ignore() -> Self {
        Self {
            act: SigAction {
                sa_handler: 1,
                sa_flags: SigActionFlags::empty(),
                sa_restore: 0,
                sa_mask: SigSet::empty(),
            },
            customed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigOp {
    Terminate,
    CoreDump,
    Ignore,
    Stop,
    Continue,
}

bitflags! {
    /// Bits in `sa_flags' used to denote the default signal action.
    pub struct SigActionFlags: usize{
    /// Don't send SIGCHLD when children stop.
        const SA_NOCLDSTOP = 1		   ;
    /// Don't create zombie on child death.
        const SA_NOCLDWAIT = 2		   ;
    /// Invoke signal-catching function with three arguments instead of one.
        const SA_SIGINFO   = 4		   ;
    /// Use signal stack by using `sa_restorer'.
        const SA_ONSTACK   = 0x08000000;
    /// Restart syscall on signal return.
        const SA_RESTART   = 0x10000000;
    /// Don't automatically block the signal when its handler is being executed.
        const SA_NODEFER   = 0x40000000;
    /// Reset to SIG_DFL on entry to handler.
        const SA_RESETHAND = 0x80000000;
    /// Historical no-op.
        const SA_INTERRUPT = 0x20000000;
    /// Use signal trampoline provided by C library's wrapper function.
        const SA_RESTORER  = 0x04000000;
    }
}

bitflags! {
    pub struct SignalStackFlags : u32 {
        const ONSTACK = 1;
        const DISABLE = 2;
        const AUTODISARM = 0x80000000;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct SignalStack {
    pub sp: usize,
    pub flags: u32,
    pub size: usize,
}

impl SignalStack {
    pub fn new(sp: usize, size: usize) -> Self {
        SignalStack {
            sp,
            flags: SignalStackFlags::DISABLE.bits,
            size,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigInfo {
    si_signo: u32,
    si_errno: u32,
    si_code: u32,
    // unsupported fields
    __pad: [u8; 128 - 3 * core::mem::size_of::<u32>()],
}

impl SigInfo {
    pub fn new(si_signo: usize, si_errno: usize, si_code: usize) -> Self {
        Self {
            si_signo: si_signo as u32,
            si_errno: si_errno as u32,
            si_code: si_code as u32,
            __pad: [0; 128 - 3 * core::mem::size_of::<u32>()],
        }
    }
}
