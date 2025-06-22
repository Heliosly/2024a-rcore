/// This module provides utility functions for the OS.
///
/// It includes functions for:
/// - Stack backtracing
/// - Page alignment
/// - Path normalization
/// - Other helper functions
use core::arch::asm;

use alloc::{borrow::ToOwned, string::String, vec::Vec};
use error::SysErrNo;
// pub use command::*;
use log::warn;

pub mod string;
pub mod error;
pub mod mm;
use crate::{config::{PAGE_SIZE, PATH_MAX}, mm::{translated_str, FrameTracker, PhysAddr, VirtAddr}, trap::TrapContext};

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

#[inline(always)]
pub const fn align_up(value: usize, align: usize) -> usize {
    assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}
/// 上对齐到页
pub fn page_round_up(v: usize) -> usize {
    if v % PAGE_SIZE == 0 {
        v
    } else {
        v - (v % PAGE_SIZE) + PAGE_SIZE
    }
}
/// 检查一个指针值是否按照指定的对齐方式对齐。
///
/// # 参数
/// - `ptr`: 要检查的指针地址（以 `usize` 表示）。
/// - `align`: 所要求的对齐字节数。
///
/// # 返回值
/// - 返回 `true` 表示 `ptr` 地址是 `align` 对齐的（即 `ptr % align == 0`）。
/// - 返回 `false` 表示未对齐。
///
/// # 示例
/// ```
/// assert_eq!(is_aligned_to(0x1000, 8), true);
/// assert_eq!(is_aligned_to(0x1003, 8), false);
/// ```
///
pub fn is_aligned_to(ptr:usize,align:usize)->bool{
    (ptr & (align - 1)) == 0
}
// 路径规范化函数
pub fn normalize_and_join_path(
    base_path: &str,
    relative_or_absolute_path: &str
) -> Result<String, SysErrNo> {
    // 1. 构造原始路径串
    //    如果传入的是绝对路径，直接使用它；
    //    否则把它拼接到 base_path 之后（保证中间只有一个 '/'）。
    let mut path_str = if relative_or_absolute_path.starts_with('/') {
        relative_or_absolute_path.to_owned()
    } else {
        let mut p = base_path.to_owned();
        if !p.ends_with('/') {
            p.push('/');
        }
        p.push_str(relative_or_absolute_path);
        p
    };

    // 2. 去除多余的连续斜杠
    //    循环替换直到不再包含 "//"
    while path_str.contains("//") && path_str.len() > 1 {
        path_str = path_str.replace("//", "/");
    }

    // 3. 分解组件，处理 "."、".."：
    //    - 空串("")：相当于多余的斜杠，忽略
    //    - "."：当前目录，
    //    - ".."：弹出上一级组件（如果存在），否则
    //         在相对路径开头保留 ".."；绝对路径在根目录多余的 ".." 忽略。
    let is_abs = path_str.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();

    for comp in path_str.split('/') {
        match comp {
            "" => {  }
            "." => {  }
            ".." => {
                if let Some(prev) = stack.pop() {
                    // 如果弹出的仍是 ".."，则需要保留两次
                    if prev == ".." {
                        stack.push(prev);
                        stack.push("..");
                    }
                } else if !is_abs {
                    // 相对路径在最前面出现 ".."，保留
                    stack.push("..");
                }
                // 绝对路径在根目录多余的 ".." 就地丢弃
            }
            other => {
                stack.push(other);
            }
        }
    }

    // 4. 重组路径
    let mut result = if is_abs { String::from("/") } else { String::new() };
    result.push_str(&stack.join("/"));

    // 5. 处理特殊结果：
    //    - 完全为空的相对路径 -> "."
    //    - 完全为空的绝对路径 -> "/"
    if result.is_empty() {
        result = String::from(".");
    } else if is_abs && result == "" {
        result = String::from("/");
    }

    // 6. 最后长度检查
    if result.len() > PATH_MAX {
        return Err(SysErrNo::ENAMETOOLONG);
    }

    Ok(result)
}
pub fn va_is_valid(va: usize,token:usize) -> bool {
    let page_table = crate::mm::PageTable::from_token(token);
    let va =VirtAddr::from(va);
    page_table.find_pte(va.clone().floor()).map(|pte| {
        let aligned_pa: PhysAddr = pte.ppn().into();
        let offset = va.page_offset();
        let aligned_pa_usize: usize = aligned_pa.into();
         let res= aligned_pa_usize + offset;
         if res <=1000{
           warn!("[translate_va] translate res<=1000,va={:#x},pte:{}",va.0,pte.bits);
           return false;
         };
        return true;
    });
    return false;
}
#[inline(never)]         // 禁止内联
#[no_mangle]            
               
#[export_name = "bpoint"] 
pub const    fn bpoint()->i32{
    let mut _a=1;
    return _a;
}
pub fn bpoint1(tf:&TrapContext) {
   
   
}
