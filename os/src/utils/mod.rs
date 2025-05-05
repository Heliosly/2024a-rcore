use core::arch::asm;

// pub use command::*;
use log::warn;

pub mod error;
use crate::{config::PAGE_SIZE, mm::VirtAddr};

/// 跟踪函数的调用栈
pub fn backtrace() {
    unsafe {
        let mut fp: usize;
        asm!("mv {}, fp", out(reg) fp);
        let mut start: VirtAddr = VirtAddr::from(fp).floor().into();
        let mut end: VirtAddr = VirtAddr::from(fp).ceil().into();
        let mut fp_addr = VirtAddr::from(fp);
        while start <= fp_addr && fp_addr < end {
            let ptr = fp as *const usize;
            warn!("[stack_backtrace] {:#x},", ptr.offset(-8).read());
            fp = ptr.offset(-16).read();
            start = VirtAddr::from(fp).floor().into();
            end = VirtAddr::from(fp).ceil().into();
            fp_addr = VirtAddr::from(fp);
        }
    }
}
/// 对齐到页
pub fn page_round_up(v: usize) -> usize {
    if v % PAGE_SIZE == 0 {
        v
    } else {
        v - (v % PAGE_SIZE) + PAGE_SIZE
    }
}


pub fn bpoint()->i32{
    let mut _a=1;
    _a +=1;
    return _a;
}
pub fn bpoint1(){
    let mut _a : i32=1;
    _a+=1;
}