use core::mem::offset_of;

use crate::{mm::{fill_str, get_target_ref_mut, translated_refmut}, task::current_token, utils::error:: SyscallRet};

use super::flags::UtsName;


const SYSNAME: &str = "rCore";
const NODENAME: &str = "rcore-host";
const RELEASE: &str = "0.1";
const VERSION: &str = "0.1 (rcore)";
const MACHINE: &str = "riscv64";
pub fn sys_uname(buf: usize) -> SyscallRet {
    let token = current_token();
    // buf 是远端虚拟地址
    let remote = buf as *mut UtsName;

    // 给每个字段调用 fill_str，把常量写到远端
    unsafe {
        // 注意：字段偏移可以用 core::ptr::addr_of_mut!
        let base = remote as *mut u8;

        // sysname: [u8; 65]
        fill_str(
            token,
            base.add(offset_of!(UtsName, sysname)),
            SYSNAME,
            65
        )?;
        // nodename: [u8; 65]
        fill_str(
            token,
            base.add(offset_of!(UtsName, nodename)),
            NODENAME,
            65
        )?;
        // release: [u8; 65]
        fill_str(
            token,
            base.add(offset_of!(UtsName, release)),
            RELEASE,
            65
        )?;
        // version: [u8; 65]
        fill_str(
            token,
            base.add(offset_of!(UtsName, version)),
            VERSION,
            65
        )?;
        // machine: [u8; 65]
        fill_str(
            token,
            base.add(offset_of!(UtsName, machine)),
            MACHINE,
            65
        )?;
    }

    Ok(0)
}