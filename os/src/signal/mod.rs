/// # 信号处理模块
///
/// 该模块实现了信号处理机制，包括信号的发送、接收、阻塞、忽略和处理。
/// 信号是一种进程间通信的方式，用于通知进程发生了某些事件。
///
/// ## 信号编号和元数据
///
/// `NSIG` 定义了支持的信号数量。
/// `SigInfo` 结构体包含了信号的编号和代码，对应于 libc 中的 `siginfo_t` 结构体。
/// `SignalStack` 结构体用于管理信号处理函数使用的栈。
/// `SigMaskHow` 枚举定义了信号掩码的操作方式，如阻塞、解除阻塞和设置掩码。
/// `Signal` 枚举定义了支持的信号类型，如 SIGHUP、SIGINT 等。
/// `SignalDefaultAction` 枚举定义了信号的默认行为，如终止进程、忽略信号等。
///
/// ## 信号处理动作
///
/// `SigAction` 结构体定义了信号处理动作，包括信号处理函数、标志和掩码。
/// `SigActionFlags` 结构体定义了信号处理动作的标志，如 SA_NOCLDSTOP、SA_SIGINFO 等。
/// `SigSet` 结构体表示一组信号，通常使用位掩码实现。
///
/// ## 任务和进程信号状态
///
/// `TaskSignalState` 结构体包含了任务的信号状态，如挂起的信号、阻塞的信号和备选信号栈。
/// `ProcessSignalSharedState` 结构体包含了进程共享的信号处理动作和挂起的信号。
///
/// ## 信号发送
///
/// `send_signal` 函数用于向目标进程或线程发送信号。
/// `send_signal_to_task` 函数用于向指定任务发送信号。
///
/// ## 信号处理
///
/// `handle_pending_signals` 函数用于处理挂起的信号，包括线程独有的和进程共享的信号。
/// `load_trap_for_signal` 函数用于恢复被信号处理函数打断前的 TrapFrame，准备返回用户态。
/// `perform_default_action_for_process` 函数用于执行信号的默认动作。
///
/// ## 系统调用
///
/// `sys_sigaction` 函数实现了 `sigaction` 系统调用，用于设置信号处理动作。
/// `sys_sigprocmask` 函数实现了 `sigprocmask` 系统调用，用于设置信号掩码。
/// `sys_kill` 函数实现了 `kill` 系统调用，用于向进程或线程发送信号。
/// `sys_sigreturn` 函数实现了 `sigreturn` 系统调用，用于从信号处理函数返回。
/// `sys_pause` 函数实现了 `pause` 系统调用，用于暂停进程的执行，直到收到信号。
///
/// ## 错误处理
///
/// `SignalError` 枚举定义了信号处理过程中可能发生的错误，如无效信号、没有找到进程或线程等。
// pub mod sigact;

//
use alloc::sync::Arc;
use bitflags::bitflags;
use core::iter::empty;
use core::{default, mem};
use num_enum::{IntoPrimitive, TryFromPrimitive};

use crate::mm::put_data;
use crate::task::current_task;
use crate::task::{ProcessControlBlock, ProcessRef, Task, TaskRef, PID2PC}; // 确保 Task 有 id()
use crate::trap::{TrapContext, UContext};
use crate::utils::error::SysErrNo; // 假设 current_task() 返回 Arc<Task>
const USER_SIGNAL_PROTECT: usize = 512;
// --- 1. 信号编号和元数据 ---
// 通常信号编号从 1 开始。0 不是有效信号。
pub const NSIG: usize = 64; // 支持的信号数量 (Linux x86_64 通常是64)
/// Signal information. Corresponds to `struct siginfo_t` in libc.
pub const SS_DISABLE: u32 = 2;

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
impl Default for SigInfo {
    fn default() -> Self {
        Self {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            __pad: [0; 128 - 3 * core::mem::size_of::<u32>()],
        }
    }
}
extern "C" {
    fn start_signal_trampoline();
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
                 // Real-time signals could be added here (SIGRTMIN to SIGRTMAX)
                 // For simplicity, we'll stick to standard signals for now.
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
        if signum <= 31 {
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
    fn get_restorer(&self) -> Option<usize> {
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
/// 向目标发送信号
/// target_pid: 目标进程的 PID
/// target_tid: 可选的目标线程的 TID (如果为 None，则发给整个进程)
/// sig: 要发送的信号
pub async fn send_signal(
    target_pid: usize,
    target_tid: Option<usize>,
    sig: Signal,
) -> Result<(), SignalError> {
    if sig as usize == 0 || sig as usize >= NSIG {
        return Err(SignalError::InvalidSignal);
    }

    // 1. 找到目标进程的 PCB
    let pcb_arc = PID2PC
        .lock() // 假设 TID2TC 映射的是 PID 到 PCB 的 Arc
        .get(&target_pid)
        .cloned()
        .ok_or(SignalError::NoSuchProcess)?;

    // TODO: 权限检查 (当前进程是否有权限向目标进程/线程发送信号) @Heliosly.
    // ...

    if let Some(tid) = target_tid {
        // --- 发送给特定线程 (tkill / pthread_kill 语义) ---
        let task_arc = pcb_arc
            .find_task_by_tid(tid)
            .await
            .ok_or(SignalError::NoSuchThread)?; // PCB 需要此方法
        let mut task_signal_state = task_arc.signal_state.lock().await;
        let process_signal_state = pcb_arc.signal_shared_state.lock(); // 需要读取 sigactions

        let action = &process_signal_state.sigactions[sig as usize];
        if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
            return Ok(()); // 忽略
        }
        task_signal_state.sigpending.add(sig);
        drop(task_signal_state);
        drop(process_signal_state);

        // 尝试唤醒目标线程 (如果它可被中断)
        // target_task_arc.try_interrupt_if_blocked(); // 需要这样的机制
        // 或者你的 wakeup_task 能处理这种情况
        let task_ptr: *const Task = Arc::as_ptr(&task_arc);
        unsafe { crate::task::waker::wakeup_task(task_ptr) }; // 假设可以安全调用
    } else {
        // --- 发送给整个进程 (kill 语义) ---
        let mut process_signal_state = pcb_arc.signal_shared_state.lock();
        let action = &process_signal_state.sigactions[sig as usize];

        if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
            return Ok(()); // 整个进程忽略此信号
        }
        process_signal_state.shared_sigpending.add(sig);
        drop(process_signal_state);

        // 选择一个合适的线程来传递这个进程信号 (或者唤醒所有可中断的线程)
        // Linux 的策略比较复杂：
        // - 如果有线程专门等待这个信号 (sigwaitinfo)，唤醒它。
        // - 否则，选择一个没有阻塞该信号的线程。
        // - 如果所有线程都阻塞了该信号，信号保持在进程挂起队列。
        // - 优先选择主线程或当前线程（如果发送给自己）。
        //
        // 简化版：唤醒进程中的一个（或所有）可被中断的线程。
        // 这可能需要迭代 pcb_arc.tasks。
        let tasks_guard = pcb_arc.tasks.lock();
        for task_ref in tasks_guard.iter() {
            // if task_ref.is_interruptible() && !task_ref.signal_thread_state.lock().sigmask.contains(sig) {
            //     task_ref.try_interrupt_if_blocked();
            //     break; // 通常只唤醒一个来处理进程信号
            // }
            // 简化：尝试唤醒第一个（或主线程）
            // 这里的唤醒是指让调度器有机会运行它，以便它能检查信号
            let task_ptr: *const Task = Arc::as_ptr(task_ref);
            unsafe { crate::task::waker::wakeup_task(task_ptr) };
            break; // 仅唤醒一个
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum SignalError {
    InvalidSignal,
    NoSuchProcess,
    NoSuchThread,
    PermissionDenied,
}

impl From<SignalError> for SysErrNo {
    fn from(err: SignalError) -> Self {
        match err {
            SignalError::InvalidSignal => SysErrNo::EINVAL,
            SignalError::NoSuchProcess => SysErrNo::ESRCH,
            SignalError::NoSuchThread => SysErrNo::ESRCH,
            SignalError::PermissionDenied => SysErrNo::EPERM,
        }
    }
}

/// 向指定任务发送一个信号（类似 `tkill`/`pthread_kill`）
/// - `task_arc`：目标任务引用
/// - `sig`：要发送的信号
/// 返回 Err 表示信号号无效或没有权限等
pub async fn send_signal_to_task(task_arc: &Arc<Task>, sig: Signal) -> Result<(), SignalError> {
    // 1. 检查信号号有效性
    let signum = sig as usize;
    if signum == 0 || signum >= NSIG {
        return Err(SignalError::InvalidSignal);
    }

    // 2. 获取进程共享的 sigactions（注册的处理方式）
    //    假设 Task 有方法 .get_pcb() 拿到它所属的进程控制块
    let pcb_arc = task_arc.get_process();
    let proc_sig_shared = pcb_arc.signal_shared_state.lock().await;
    let action = proc_sig_shared.sigactions[signum];
    // SIGKILL 和 SIGSTOP 永远不能被忽略
    if action.handler == SIG_IGN && sig != Signal::SIGKILL && sig != Signal::SIGSTOP {
        return Ok(());
    }
    drop(proc_sig_shared);

    // 3. 将信号加入该线程的挂起集合
    {
        let mut task_sig_state = task_arc.signal_state.lock().await;
        task_sig_state.sigpending.add(sig);
    }

    // 4. 唤醒该任务，让调度器有机会运行它，
    //    以便它在回用户态前调用 handle_pending_signals
    let task_ptr: *const Task = Arc::as_ptr(task_arc);
    crate::task::waker::wakeup_task(task_ptr);

    Ok(())
}
pub fn handle_pending_signals(task_arc: &Arc<Task>, pcb_arc: &Arc<ProcessControlBlock>) {
    // 1. 获取线程和进程的信号状态锁
    let mut task_state = task_arc.signal_state.lock();
    let mut process_state = pcb_arc.signal_shared_state.lock();
    let token = unsafe { *task_arc.page_table_token.get() };
    loop {
        // 可能有多个信号需要处理
        let mut signal_to_deliver: Option<Signal> = None;
        let mut delivered_from_thread_pending = false;

        // a. 优先检查并处理线程独有的、未被阻塞的挂起信号
        for signum_idx in 1..NSIG {
            if let Some(sig) = Signal::from_usize(signum_idx) {
                if task_state.sigpending.contains(sig) && !task_state.sigmask.contains(sig) {
                    signal_to_deliver = Some(sig);
                    delivered_from_thread_pending = true;
                    break;
                }
            }
        }

        // b. 如果没有线程独有的，则检查进程共享的、未被此线程阻塞的挂起信号
        if signal_to_deliver.is_none() {
            for signum_idx in 1..NSIG {
                if let Some(sig) = Signal::from_usize(signum_idx) {
                    if process_state.shared_sigpending.contains(sig)
                        && !task_state.sigmask.contains(sig)
                    {
                        signal_to_deliver = Some(sig);
                        delivered_from_thread_pending = false; // 来自进程队列
                        break;
                    }
                }
            }
        }

        if let Some(sig) = signal_to_deliver {
            let action = process_state.sigactions[sig as usize].clone(); // 动作是进程共享的

            // 从相应的挂起队列中移除
            if delivered_from_thread_pending {
                task_state.sigpending.remove(sig);
            } else {
                process_state.shared_sigpending.remove(sig);
                // 重要：如果这个信号是发给进程的，理论上只有一个线程会处理它。
                // 其他线程不应该再看到这个进程挂起信号（除非是广播信号或特殊情况）。
                // 我们的模型是，一旦一个线程选中了一个进程信号来传递，就从共享队列移除。
            }

            // 特殊处理 SIGKILL 和 SIGSTOP (它们不能被捕获或忽略，动作是固定的)
            if sig == Signal::SIGKILL {
                // TODO: 终止整个进程 (所有线程) @Heliosly.
                // log::info!("Process {} (task {}) received SIGKILL, terminating all tasks.", pcb_arc.pid.0, task_arc.id());
                // pcb_arc.terminate_all_tasks_and_self(); // 假设有此方法
                return; // 进程终止，无需继续
            }
            if sig == Signal::SIGSTOP {
                // TODO: 停止整个进程 (所有线程) @Heliosly.
                // log::info!("Process {} (task {}) received SIGSTOP, stopping all tasks.", pcb_arc.pid.0, task_arc.id());
                // pcb_arc.stop_all_tasks(); // 假设有此方法
                // SIGSTOP 后，此线程也应停止，可能不会继续循环。或者由调度器处理。
                // 这里我们先假设它处理完这个信号后，如果还有其他信号，可以继续。
                // 但通常 SIGSTOP 会导致任务状态改变，不再执行。
                // 为了简单，我们先 continue loop。
                continue;
            }
            // 同样，SIGCONT 需要唤醒进程中的所有线程
            if sig == Signal::SIGCONT {
                // log::info!("Process {} (task {}) received SIGCONT, continuing all tasks.", pcb_arc.pid.0, task_arc.id());
                // pcb_arc.continue_all_tasks(); // 假设有此方法
                // SIGCONT 的默认动作是 Ignore (如果之前是Stop) 或 Continue。
                // 如果有用户处理器，则执行用户处理器。
                // 我们这里先按默认处理，如果用户有处理器，下面会走到。
            }

            // 计算在信号处理函数执行期间需要阻塞的掩码
            let mut new_mask_during_handler = task_state.sigmask; // 基于线程当前掩码
            if !action.flags.contains(SigActionFlags::SA_NODEFER) {
                new_mask_during_handler.add(sig);
            }
            new_mask_during_handler.union_with(&action.mask); // 加上 sa_mask

            // 释放锁，因为执行动作或准备用户态帧可能需要时间，或发生上下文切换
            let original_thread_mask = task_state.sigmask; // 保存原始掩码以备恢复
            task_state.sigmask = new_mask_during_handler; // 临时设置新掩码
            drop(task_state);
            drop(process_state); // 释放两个锁

            // -- 执行动作 --
            match action.handler {
                SIG_DFL => {
                    unsafe { perform_default_action_for_process(pcb_arc, task_arc, sig) };
                    // 默认动作可能影响整个进程
                }
                SIG_IGN => { /* 忽略 */ }
                user_handler_addr => {
                    // log::info!("Task {} (in process {}) delivering signal {:?} to handler 0x{:x}",
                    //          task_arc.id(), pcb_arc.pid.0, sig, user_handler_addr);
                    let sig_num: usize = sig.into();
                    // 此时需要调用信号处理函数，注意调用的方式是：
                    // 通过修改trap上下文的pc指针，使得trap返回之后，直接到达信号处理函数
                    // 因此需要处理一系列的trap上下文，使得正确传参与返回。
                    // 具体来说需要考虑两个方面：
                    // 1. 传参
                    // 2. 返回值ra地址的设定，与是否设置了SA_RESTORER有关

                    // 读取当前的trap上下文
                    // let mut trap_frame = read_trapframe_from_kstack(current_task.get_kernel_stack_top().unwrap());
                    let mut task_state = task_arc.signal_state.lock();

                    let trap_frame = task_arc.get_trap_cx().unwrap();

                    // // 新的trap上下文的sp指针位置，由于SIGINFO会存放内容，所以需要开个保护区域
                    let mut sp = if action.flags.contains(SigActionFlags::SA_ONSTACK)
                        && task_state.alternate_stack.flags != SS_DISABLE
                    {
                        debug!("Use alternate stack");
                        // Use alternate stack
                        (task_state.alternate_stack.sp + task_state.alternate_stack.size - 1) & !0xf
                    } else {
                        trap_frame.get_sp() - USER_SIGNAL_PROTECT
                    };

                    info!("use stack: {:#x}", sp);
                    let restorer = if let Some(addr) = action.get_restorer() {
                        addr
                    } else {
                        start_signal_trampoline as usize
                    };

                    info!(
                        "restorer :{:#x}, handler: {:#x}",
                        restorer, user_handler_addr
                    );
                    #[cfg(not(target_arch = "x86_64"))]
                    trap_frame.set_ra(restorer);

                    let old_pc = trap_frame.get_pc();

                    trap_frame.set_pc(user_handler_addr);
                    // 传参
                    trap_frame.set_arg0(sig.into());
                    // 若带有SIG_INFO参数，则函数原型为fn(sig: SignalNo, info: &SigInfo, ucontext: &mut UContext)
                    if action.flags.contains(SigActionFlags::SA_SIGINFO) {
                        // current_task.set_siginfo(true);
                        task_state.sig_info = true;
                        let sp_base = (((sp - core::mem::size_of::<SigInfo>()) & !0xf)
                            - core::mem::size_of::<UContext>())
                            & !0xf;

                        // TODO: 统一为访问用户空间的操作封装函数
                        // process
                        //     .manual_alloc_range_for_lazy(sp_base.into(), sp.into())
                        //     .await
                        //     .expect("Failed to alloc memory for signal user stack");

                        // 注意16字节对齐
                        sp = (sp - core::mem::size_of::<SigInfo>()) & !0xf;
                        let info = SigInfo {
                            si_signo: sig_num as u32,
                            ..Default::default()
                        };
                        unsafe {
                            *(sp as *mut SigInfo) = info;
                        }
                        trap_frame.set_arg1(sp);

                        // 接下来存储ucontext
                        sp = (sp - core::mem::size_of::<UContext>()) & !0xf;

                        let ucontext = UContext::init(old_pc, original_thread_mask);
                        put_data(token, sp as *mut UContext, ucontext);

                        trap_frame.set_arg2(sp);
                    }

                    trap_frame.set_sp(sp);
                    if action.flags.contains(SigActionFlags::SA_RESETHAND) {
                        let mut temp_proc_state = pcb_arc.signal_shared_state.lock();
                        temp_proc_state.sigactions[sig as usize].handler = SIG_DFL;
                    }
                    return; // 信号已交付给用户处理程序，本次内核处理结束
                }
            }
            // 如果执行到这里（例如 SIG_IGN 或某些不终止的 SIG_DFL），重新获取锁并继续循环
            task_state = task_arc.signal_state.lock();
            process_state = pcb_arc.signal_shared_state.lock();
            task_state.sigmask = original_thread_mask; // 恢复掩码（如果没进用户处理函数）
        } else {
            // 没有需要处理的信号了
            // drop(thread_state); // 在循环外统一 drop
            // drop(process_state);
            break;
        }
    }
    // 确保锁在这里被释放
}

/// 恢复被信号处理函数打断前的 TrapFrame，准备返回用户态。
/// 如果存在 saved_trap，则返回 true（表示已装载），否则返回 false。
#[no_mangle]
pub async fn load_trap_for_signal() -> bool {
    // 1. 找到当前任务
    let task = current_task();
    // 2. 拿到它的信号状态
    let mut sig_state = task.signal_state.lock().await;

    // 3. 如果之前有保存的 TrapFrame，就拿出来
    if let Some(saved_frame) = sig_state.last_context.take() {
        unsafe {
            // 4. 拿到实际用于中断/异常的内核栈上的 TrapFrame 指针
            let now_trap_frame: &mut TrapContext = task.get_trap_cx().unwrap();

            // 5. 拷贝回去，恢复原先全部寄存器状态
            *now_trap_frame = saved_frame;

            // 6. 如果当时用的是 SA_SIGINFO（sig_info = true），
            //    需要从用户栈上的 UContext 里再拿一次 PC
            if sig_state.sig_info {
                // 用户态信号上下文结构在用户栈顶
                let sp = now_trap_frame.regs.sp;
                let user_ctx = &*(sp as *const UContext);
                let pc = user_ctx.get_pc();
                now_trap_frame.set_pc(pc);
            }
        }
        true
    } else {
        // 没有未处理的信号上下文
        false
    }
}

fn perform_default_action_for_process(
    pcb_arc: &ProcessRef,
    _current_task_arc: &TaskRef,
    sig: Signal,
) {
    // 默认动作现在可能需要作用于整个进程
    match sig.default_action() {
        SignalDefaultAction::Terminate | SignalDefaultAction::CoreDump => {
            // log::info!("Process {} terminating due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.terminate_all_tasks_and_self();
        }
        SignalDefaultAction::Ignore => {}
        SignalDefaultAction::Stop => {
            // log::info!("Process {} stopping due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.stop_all_tasks();
        }
        SignalDefaultAction::Continue => {
            // log::info!("Process {} continuing due to signal {:?}", pcb_arc.pid.0, sig);
            // pcb_arc.continue_all_tasks();
        }
        SignalDefaultAction::ForceTerminateOrStop => {
            unreachable!("SIGKILL/SIGSTOP default actions should be handled earlier in handle_pending_signals");
        }
    }
}

// // --- 系统调用实现 (接口和核心逻辑) ---
// // 这些函数是内核的入口点，需要进行参数校验（如指针有效性）

// // int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
// pub async fn sys_sigaction(
//     signum_usize: usize,
//     act_user_ptr: *const SigAction,
//     oldact_user_ptr: *mut SigAction,
// ) -> SyscallRet {
//     let sig = match Signal::from_usize(signum_usize) {
//         Some(s) => s,
//         None => return Err(SysErrNo::EINVAL), // 无效信号
//     };

//     // SIGKILL 和 SIGSTOP 的动作不能被改
//     if sig == Signal::SIGKILL || sig == Signal::SIGSTOP {
//         return Err(SysErrNo::EINVAL);
//     }

//     let mut process = current_process();
//     let mut shared_state = process.signal_shared_state.lock().await;

//     // 如果 oldact 非空，保存旧的动作
//     // let mut old_action_to_return: SigAction = SigAction::default();
//     if !oldact_user_ptr.is_null() {
//         // TODO: 实现从内核复制到用户空间 (copy_to_user)  @Heliosly.
//         // 需要验证 oldact_user_ptr 的有效性
//         // let old_action = signal_state.sigactions[sig as usize];
//         // unsafe { copy_to_user(oldact_user_ptr, &old_action)? };
//         // 简化：假设可以直接写，或此函数在内核中，不直接处理用户指针
//         // 在OS比赛中，你可能需要实现 copy_to_user 和 copy_from_user
//         // 暂时我们只在内核数据结构中操作
//         // 返回值是旧的action，让用户空间处理复制
//         *translated_refmut(process.get_user_token().await, oldact_user_ptr)? =
//             shared_state.sigactions[sig as usize];
//     }

//     // 如果 act 非空，设置新的动作
//     if !act_user_ptr.is_null() {
//         // TODO: 实现从用户空间复制到内核 (copy_from_user) @Heliosly.
//         // 需要验证 act_user_ptr 的有效性
//         // let new_action = unsafe { copy_from_user(act_user_ptr)? };
//         // 简化：假设可以直接读
//         let new_action = unsafe { *act_user_ptr }; // 极度不安全，仅为结构示意

//         // 校验 new_action 的合法性 (例如 handler 地址)
//         // ...

//         shared_state.sigactions[sig as usize] = new_action;
//         // log::debug!("Task {} set action for signal {:?} to handler 0x{:x}", current_task_arc.id(), sig, new_action.handler);
//     }

//     // 如果 oldact_user_ptr 非空，需要将 old_action_to_return 写回用户空间
//     // if !oldact_user_ptr.is_null() {
//     //     // TODO: copy_to_user(oldact_user_ptr, &old_action_to_return)  @Heliosly.
//     //     // 假设能直接写，这是不安全的：
//     //     unsafe { *oldact_user_ptr = old_action_to_return; }
//     // }

//     Ok(0) // 成功
// }

// // int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
// pub fn sys_sigprocmask(
//     how: i32,
//     set_user_ptr: *const SigSet,
//     oldset_user_ptr: *mut SigSet,
// ) -> SyscallRet {
//     let current_task_arc = current_task();
//     let mut signal_state = current_task_arc.signal_state.lock();

//     let old_mask = signal_state.sigmask;

//     if !set_user_ptr.is_null() {
//         // TODO: copy_from_user for set
//         let set = unsafe { *set_user_ptr }; // 不安全，仅为示意

//         let enum_how = match SigMaskHow::try_from(how) {
//             Ok(how) => how,
//             Err(_) => return Err(SysErrNo::EINVAL),
//         };
//         match enum_how {
//             SigMaskHow::SIG_BLOCK => {
//                 signal_state.sigmask.union_with(&set);
//             }
//             SigMaskHow::SIG_UNBLOCK => {
//                 // 不能解除 SIGKILL 或 SIGSTOP 的阻塞 (虽然它们通常不被计入掩码)
//                 // 但 SigSet 的操作通常不关心具体信号的特殊性，这是更高层逻辑
//                 let mut temp_set = set;
//                 // Linux 不允许 SIGKILL 和 SIGSTOP 被阻塞，所以从 set 中移除它们
//                 temp_set.remove(Signal::SIGKILL);
//                 temp_set.remove(Signal::SIGSTOP);
//                 // 然后从当前掩码中移除这些（解除阻塞）
//                 // A = A & (~B) => A.intersect_with( !B )
//                 // SigSet 需要实现 bitwise NOT 或者一个 remove_all_from_set 方法
//                 // 简单做法：迭代 set 中的每一位，如果在 sigmask 中，则 remove
//                 // (更正：SIG_UNBLOCK 是移除 set 中的位，所以是 sigmask &= ~set)
//                 // sigmask = sigmask AND (NOT set)
//                 // 我们需要一个 SigSet::complement() 或 SigSet::difference_with()
//                 // 简化：直接迭代要解除阻塞的信号
//                 for i in 1..NSIG {
//                     if let Some(s) = Signal::from_usize(i) {
//                         if set.contains(s) {
//                             signal_state.sigmask.remove(s);
//                         }
//                     }
//                 }
//             }
//             SigMaskHow::SIG_SETMASK => {
//                 let mut new_mask = set;
//                 // 不能阻塞 SIGKILL 或 SIGSTOP
//                 new_mask.remove(Signal::SIGKILL);
//                 new_mask.remove(Signal::SIGSTOP);
//                 signal_state.sigmask = new_mask;
//             }
//         }
//     }

//     drop(signal_state); // 先释放锁，再复制到用户空间

//     if !oldset_user_ptr.is_null() {
//         // TODO: copy_to_user for old_mask
//         unsafe {
//             *oldset_user_ptr = old_mask;
//         }
//     }

//     Ok(0) // 成功
// }

// // int kill(pid_t pid, int sig); (或 tkill/tgkill)
// pub fn sys_kill(target_tid: usize, signum_usize: usize) -> SyscallRet {
//     let sig = match Signal::from_usize(signum_usize) {
//         Some(s) => s,
//         None => return Err(SysErrNo::EINVAL), // 无效信号
//     };

//     if signum_usize == 0 {
//         // 发送信号0是检查进程是否存在，不实际发送信号

//         if TID2TC.lock().contains_key(&target_tid) {
//             return Ok(0); // 存在
//         } else {
//             return Err(SysErrNo::ESRCH); // 不存在
//         }
//     }

//     let target_task_arc = match TID2TC.lock().get(&target_tid) {
//         Some(task_ref) => task_ref.clone(),
//         None => return Err(SysErrNo::ESRCH), // No such process/task
//     };

//     // TODO: 权限检查 (例如，当前任务是否有权限向目标任务发送信号)  @Heliosly.
//     // ...

//     send_signal_to_task(&target_task_arc, sig);
//     Ok(0) // 成功 (信号已加入挂起队列或被处理)
// }

// // int pause(void);
// pub fn sys_pause() -> SyscallRet {
//     let task_arc = current_task();
//     // 1. 将当前任务的信号掩码保存起来 (old_mask = task.sigmask)。
//     // 2. 将当前任务的信号掩码设置为空 (允许所有信号)。
//     //    或者使用一个临时的空掩码。
//     // 3. 使任务进入可中断的睡眠状态，直到一个信号被捕获并处理。
//     //    这通常通过 `sigsuspend(empty_mask)` 实现。
//     //    这里我们简化：
//     //    - 检查是否有未阻塞的挂起信号，如果有，handle_pending_signals 会处理，pause 不会阻塞。
//     //    - 如果没有，则阻塞，等待任何信号。
//     //
//     // loop {
//     //     handle_pending_signals(&task_arc); // 处理已有的信号
//     //     // 如果信号导致任务终止或停止，就不会到这里
//     //
//     //     // 让任务睡眠，等待被信号唤醒
//     //     // 这需要一种机制，比如一个信号专用的条件变量或等待队列
//     //     // task_arc.sleep_interruptible_until_signal();
//     //     // 如果被唤醒，说明有信号传递，handle_pending_signals 会在返回用户态前再次运行。
//     //     // pause 的返回值总是 -EINTR (被信号中断)。
//     //     // 如果一个信号处理函数被执行了，pause 就返回了。
//     //     // 如果信号的动作是终止，pause 就不会返回。
//     //
//     //     // 这里需要一个方法让当前任务阻塞，直到有信号处理。
//     //     // 一个简单的方法是设置一个flag，然后yield，直到flag被信号处理机制改变。
//     //     // 或者，使用一个 Waker，当有信号时被唤醒。
//     //     // 这部分逻辑比较复杂，通常与 sigsuspend 紧密相关。
//     // }
//     // 对于OS比赛，pause 可以简化为：检查并处理当前挂起信号，
//     // 如果没有导致返回的信号（例如，被忽略的或导致终止的），
//     // 则让任务进入一个可被任何信号唤醒的阻塞状态。
//     // `pause` 总是返回 -EINTR，除非被一个导致进程终止的信号打断。
//     Err(SysErrNo::EINTR) // 表示被信号中断
// }

// // 你需要在你的 Task 结构中添加 `signal_state: Mutex<TaskSignalState>`
// // 并在 Task 创建时初始化它：
// // impl Task {
// //     pub fn new(id: usize, /* ... other args ... */) -> Arc<Self> {
// //         Arc::new(Self {
// //             // ...
// //             id,
// //             status: Mutex::new(TaskStatus::Runnable),
// //             signal_state: Mutex::new(TaskSignalState::default()),
// //             // ...
// //         })
// //     }
// // }
