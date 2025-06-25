//! RISC-V timer-related functionality

use core::ops::{Add, Sub};

use crate::config::CLOCK_FREQ;
use crate::sbi::set_timer;
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
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
    #[cfg(target_arch = "riscv64")]
    {
        time::read()
    }
    #[cfg(target_arch = "loongarch64")]
    {
        let time: u64;
        unsafe {
            core::arch::asm!("rdtime.d {}, $zero", out(reg) time);
        }
        time as usize
    }
}

/// Get current time in milliseconds
pub fn get_time_ms() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        time::read() * MSEC_PER_SEC / CLOCK_FREQ
    }
    #[cfg(target_arch = "loongarch64")]
    {
        let counter = get_time();
        counter * MSEC_PER_SEC / CLOCK_FREQ
    }
}
#[repr(C)]
#[derive(Debug,PartialEq, Eq,PartialOrd, Ord,Clone, Copy)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

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
    #[cfg(target_arch = "riscv64")]
    {
        time::read() * MICRO_PER_SEC / CLOCK_FREQ
    }
    #[cfg(target_arch = "loongarch64")]
    {
        let counter = get_time();
        counter * MICRO_PER_SEC / CLOCK_FREQ
    }
}

/// Get current time in nanoseconds
pub fn get_time_ns() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        let ticks = time::read() as u128;
        let nanos = ticks * (NANO_PER_SEC as u128) / (CLOCK_FREQ as u128);
        nanos as usize
    }
    #[cfg(target_arch = "loongarch64")]
    {
        let ticks = get_time() as u128;
        let nanos = ticks * (NANO_PER_SEC as u128) / (CLOCK_FREQ as u128);
        nanos as usize
    }
}

pub fn get_time_ticks() -> u128 {
    #[cfg(target_arch = "riscv64")]
    {
        time::read() as u128
    }
    #[cfg(target_arch = "loongarch64")]
    {
        get_time() as u128
    }
}

pub fn get_time_ticks_us() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        time::read()
    }
    #[cfg(target_arch = "loongarch64")]
    {
        get_time()
    }
}
/// Set the next timer interrupt
pub fn set_next_trigger() {
    set_timer(get_time() + CLOCK_FREQ / TICKS_PER_SEC);
}

pub fn get_usertime() -> UserTimeSpec {
    let ticks = get_time_ticks();
    let tv_sec = (ticks / CLOCK_FREQ as u128) as usize;
    let tv_nsec = ((ticks % CLOCK_FREQ as u128) * NANO_PER_SEC as u128 / CLOCK_FREQ as u128) as usize;
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

