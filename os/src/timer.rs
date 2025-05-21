//! RISC-V timer-related functionality

use core::ops::{Add, Sub};

use crate::config::CLOCK_FREQ;
use crate::sbi::set_timer;
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
    time::read()
}

/// Get current time in milliseconds
pub fn get_time_ms() -> usize {
    time::read() * MSEC_PER_SEC / CLOCK_FREQ
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
}

impl Add for TimeVal {
    type Output = TimeVal;

    fn add(self, other: TimeVal) -> TimeVal {
        let sec = self.sec + other.sec;
        let usec = self.usec + other.usec;
        TimeVal::normalize(sec, usec)
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
    time::read() * NANO_PER_SEC / CLOCK_FREQ
}

/// Set the next timer interrupt
pub fn set_next_trigger() {
    set_timer(get_time() + CLOCK_FREQ / TICKS_PER_SEC);
}
