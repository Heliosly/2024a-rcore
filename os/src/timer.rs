//! RISC-V timer-related functionality

use core::ops::{Add, Sub};

use crate::config::CLOCK_FREQ;
use crate::sbi::set_timer;
use crate::signal::{send_signal, Signal};
use crate::sync::Mutex;
use crate::task::PID2PC;
use alloc::collections::BTreeMap;
use lazy_init::LazyInit;
use riscv::register::time;

/// The number of ticks per second
const TICKS_PER_SEC: usize = 100;
/// The number of milliseconds per second
const MSEC_PER_SEC: usize = 1_000;
/// The number of microseconds per second
const MICRO_PER_SEC: usize = 1_000_000;
/// The number of nanoseconds per second
const NANO_PER_SEC: usize = 1_000_000_000;

/// Get the current time in ticks
pub fn get_time() -> usize {
    time::read() / CLOCK_FREQ
}
pub fn get_time_ticks() -> usize {
    time::read()
}

/// Get current time in milliseconds
pub fn get_time_ms() -> usize {
    time::read() * MSEC_PER_SEC / CLOCK_FREQ
}
#[repr(C)]
#[derive(Debug,PartialEq, Eq,PartialOrd, Ord,Clone, Copy,Default)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}
const NSEC_PER_USEC: u64 = 1_000;
pub fn current_time() -> TimeVal {
    let time_ns = get_time_ns();
    TimeVal {
        sec: time_ns / NANO_PER_SEC,
        usec: (time_ns % NANO_PER_SEC) / 1000,
    }
}
impl TimeVal {
    ///仅加法可用
   pub  fn add_milliseconds(&self, ms: usize) -> TimeVal {
        let sec = self.sec + ms / MSEC_PER_SEC;
        let usec = self.usec + (ms % MSEC_PER_SEC) * 1000;
        TimeVal::normalize(sec, usec)
    }
    fn new(sec: usize, usec: usize) -> Self {
        Self { sec, usec }
    }
    /// 将 `usec` 规范化到 [0, 1_000_000) 范围，并把溢出微秒转换到秒里
    fn normalize(mut sec: usize, mut usec: usize) -> TimeVal {
        // 当 usec >= 1_000_000 时进位
        if usec >= 1_000_000 {
            sec += usec / 1_000_000;
            usec %= 1_000_000;
        }
        TimeVal { sec, usec }
    }

        pub fn add_timespec(&self, ts: &UserTimeSpec) -> Self {
            let sec = self.sec + ts.tv_sec as usize;
            let usec = self.usec + ts.tv_nsec as usize / 1000;
           TimeVal::normalize(sec,usec)
        }
    pub fn from_ns(time:u64)->Self{
        TimeVal {
            sec: (time / NANO_PER_SEC as u64) as usize,
            usec: ((time % NANO_PER_SEC as u64) / 1000) as usize,
        }
    }
}

impl Add for TimeVal {
    type Output = TimeVal;

    fn add(self, other: TimeVal) -> TimeVal {
        let sec = self.sec + other.sec;
        let usec = self.usec + other.usec;
        TimeVal::normalize(sec, usec)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default,PartialEq, Eq, PartialOrd, Ord)]
pub struct UserTimeSpec { // 对应 struct timespec
    pub tv_sec: usize,  // seconds
    pub tv_nsec: usize, // nanoseconds (long)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelTimer {
    pub interval: u64, // 间隔时间，单位：纳秒
    pub value: u64,    // 剩余时间，单位：纳秒
}
impl KernelTimer {
    /// 检查定时器是否已启动
    pub fn is_armed(&self) -> bool {
        self.value > 0
    }
}
impl From<TimeVal> for u64 {
    /// 从 TimeVal 转换为纳秒
    fn from(tv: TimeVal) -> Self {
        (tv.sec as u64 * NANO_PER_SEC as u64) + (tv.usec as u64 * NSEC_PER_USEC)
    }
}
impl UserTimeSpec{
    pub fn as_nanos(&self) -> usize {
        self.tv_sec * NANO_PER_SEC + self.tv_nsec
    }
}
pub fn usertime2_timeval(usertime :&UserTimeSpec)->TimeVal{
       
TimeVal{
    sec:usertime.tv_sec,
    usec:usertime.tv_nsec/1000,
}
}
impl Sub for UserTimeSpec {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        // 确保 self >= other，避免负数
        if self >= other {
            let mut sec = self.tv_sec - other.tv_sec;
            let mut nsec = self.tv_nsec as isize - other.tv_nsec as isize;

            if nsec < 0 {
                sec -= 1;
                nsec += NANO_PER_SEC as isize;
            }

            Self {
                tv_sec: sec,
                tv_nsec: nsec as usize,
            }
        } else {
            // 如果 self < other，返回 0
            Self {
                tv_sec: 0,
                tv_nsec: 0,
            }
        }
    }
}

impl Add for UserTimeSpec {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let mut sec = self.tv_sec + other.tv_sec;
        let mut nsec = self.tv_nsec + other.tv_nsec;
        if nsec >= NANO_PER_SEC {
            sec += nsec / NANO_PER_SEC;
            nsec %= NANO_PER_SEC;
        }
        Self {
            tv_sec: sec,
            tv_nsec: nsec,
        }
    }
}

impl Sub for TimeVal {
    type Output = TimeVal;

    fn sub(self, other: TimeVal) -> TimeVal {
        // 为了避免负值，这里假设 self >= other
        let (sec, usec) =
            if self.sec > other.sec || (self.sec == other.sec && self.usec >= other.usec) {
                // 正常情况
                let mut sec = self.sec - other.sec;
                let mut usec = if self.usec >= other.usec {
                    self.usec - other.usec
                } else {
                    // 借一秒
                    sec = sec.saturating_sub(1);
                    self.usec + 1_000_000 - other.usec
                };
                (sec, usec)
            } else {
                // 如果 self < other，就返回 0
                (0, 0)
            };
        TimeVal { sec, usec }
    }
}

/// Get current time in microseconds
pub fn get_time_us() -> usize {
    time::read() * MICRO_PER_SEC / CLOCK_FREQ
}

/// Get current time in nanoseconds
pub fn get_time_ns() -> usize {
    let ticks = time::read() as u128;
    let nanos = ticks * (NANO_PER_SEC as u128) / (CLOCK_FREQ as u128);
    nanos as usize
}


/// Set the next timer interrupt
pub fn set_next_trigger() {
    set_timer(get_time_ticks() + CLOCK_FREQ / TICKS_PER_SEC);
}

pub fn get_usertime() -> UserTimeSpec {
    let ticks= get_time_ticks();
    let tv_sec= ticks / CLOCK_FREQ;
    let tv_nsec= (ticks % CLOCK_FREQ) * NANO_PER_SEC / CLOCK_FREQ;
    UserTimeSpec { tv_sec, tv_nsec }

}

#[repr(C)]
#[derive(Default,Clone, Copy)]
pub struct Tms {
    pub tms_utime: isize,  //用户模式下花费的CPU时间
    pub tms_stime: isize,  //内核模式下花费的CPU时间
    pub tms_cutime: isize, //子进程在用户模式下花费的CPU时间
    pub tms_cstime: isize, //子进程在内核模式下花费的CPU时间
}

impl Tms {
    pub fn new(time_data: &TimeData) -> Self {
        Self {
            tms_utime: time_data.utime,
            tms_stime: time_data.stime,
            tms_cutime: time_data.cutime,
            tms_cstime: time_data.cstime,
        }
    }
}

#[derive(Clone,Copy)]
pub struct TimeData {
    pub utime: isize,//用户模式下花费的CPU时间
    pub stime: isize, //内核模式下花费的CPU时间
    pub cutime: isize, //子进程在用户模式下花费的CPU时间
    pub cstime: isize,//子进程在内核模式下花费的CPU时间
    pub lasttime: isize,
}

impl Default for TimeData{
    fn default() -> Self {
        let now = (get_time_ms()) as isize;
        Self {
            utime: 0,
            stime: 0,
            cutime: 0,
            cstime: 0,
            lasttime: now,
        }
    }
}
impl TimeData {
    
    pub fn new() -> Self {
        let now = (get_time_ms()) as isize;
        Self {
            utime: 0,
            stime: 0,
            cutime: 0,
            cstime: 0,
            lasttime: now,
        }
    }
    pub fn update_utime(&mut self) {
        let now = (get_time_ms()) as isize;
        let duration = now - self.lasttime;
        self.utime += duration;
        self.lasttime = now;
    }
    pub fn update_stime(&mut self) {
        let now = (get_time_ms()) as isize;
        let duration = now - self.lasttime;
        self.stime += duration;
        self.lasttime = now;
    }
    pub fn clear(&mut self) {
        let now = (get_time_ms()) as isize;
        self.utime = 0;
        self.stime = 0;
        self.cutime = 0;
        self.cstime = 0;
        self.lasttime = now;
    }
}

static REAL_TIMERS: LazyInit<Mutex<BTreeMap<u64, usize>>> = LazyInit::new();

pub fn init_timer_backend() {
    REAL_TIMERS.init_by(Mutex::new(BTreeMap::new()));
}

/// 当 sys_setitimer 设置 ITIMER_REAL 时调用此函数
pub async  fn set_real_timer(pid: usize, value_ns: u64) {
    let mut timers = REAL_TIMERS.lock().await;
    // 首先移除该进程可能存在的旧定时器
    timers.retain(|_, p| *p != pid);
    
    // 如果 value > 0，说明是启动定时器
    if value_ns > 0 {
        let deadline = get_time_ns() as u64 + value_ns; // get_time_ns() 获取当前时间
        timers.insert(deadline, pid);
    }
}
pub const ITIMER_REAL: i32 = 0;
pub const ITIMER_VIRTUAL: i32 = 1;
pub const ITIMER_PROF: i32 = 2;

/// 在每个时钟中断处理函数的末尾调用
pub async  fn check_real_timers() {
    let mut timers = REAL_TIMERS.lock().await;
    let now = get_time_ns();

    while let Some((&deadline, &pid)) = timers.iter().next() {
        if deadline > now as u64 {
            break; // 最早的定时器还没到期，后面的肯定也没到
        }
        
        // 定时器到期，从队列中移除
        timers.pop_first();

        // 找到进程并处理
        if let Some(process) = PID2PC.lock().get(&pid) {
            let mut real_timer = process.timers[ITIMER_REAL as usize].lock().await;

            // 发送信号
            send_signal(pid, None, crate::signal::Signal::SIGALRM).await.unwrap();

            // 如果是周期性定时器，重新设置并加入队列
            if real_timer.interval > 0 {
                real_timer.value = real_timer.interval;
                let next_deadline = now as u64 + real_timer.interval;
                timers.insert(next_deadline as u64, pid);
            } else {
                // 一次性定时器，清零
                real_timer.value = 0;
            }
        }
    }
}
pub async fn handle_timer_tick() {
    const NSEC_PER_SEC: u64 = 1_000_000_000;
const TICK_FREQUENCY: u64 = 100; // 假设是 100Hz
const NSEC_PER_TICK: u64 = NSEC_PER_SEC / TICK_FREQUENCY;
    // 1. 更新当前进程的 TimeData (您的内核应该已经有这部分逻辑了)
    let process = crate::task::current_process();


    // 2. ★★★ 新增：驱动 CPU 时间定时器 ★★★

    // --- 驱动 ITIMER_VIRTUAL (只消耗用户时间) ---
        let mut vtimer = process.timers[ITIMER_VIRTUAL as usize].lock().await;
        if vtimer.value > 0 { // 如果定时器已启动
            if vtimer.value <= NSEC_PER_TICK {
                // 时间到！
                send_signal(process.get_pid(), None, crate::signal::Signal::SIGVTALRM).await.unwrap();
                vtimer.value = vtimer.interval; // 重置为间隔值或清零
            } else {
                vtimer.value -= NSEC_PER_TICK; // 倒计时
            }
        }
    ;
    // --- 驱动 ITIMER_PROF (消耗用户和内核时间) ---
    let mut ptimer = process.timers[ITIMER_PROF as usize].lock().await;
    if ptimer.value > 0 { // 如果定时器已启动
        if ptimer.value <= NSEC_PER_TICK {
            // 时间到！
            send_signal(process.get_pid(), None, crate::signal::Signal::SIGPROF).await.unwrap();
            ptimer.value = ptimer.interval; // 重置
        } else {
            ptimer.value -= NSEC_PER_TICK; // 倒计时
        }
    }

    
    check_real_timers().await; 
}