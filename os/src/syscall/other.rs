use core::mem::offset_of;

use linux_raw_sys::general::MAX_CLOCKS;

use crate::{mm::{fill_str,  put_data}, syscall::flags::Utsname, task::current_token, timer::{self, UserTimeSpec}, utils::error::{SysErrNo,  SyscallRet}};




pub async  fn sys_uname(buf: usize) -> SyscallRet {
    trace!("[sys_uname]");
    fn str2u8(s: &str) -> [u8; 65] {
        let mut b = [0; 65];
        b[0..s.len()].copy_from_slice(s.as_bytes());
        b
    }
    let uname = Utsname {
        sysname: str2u8("Linux"),
        nodename: str2u8("Rcore"),
        release: str2u8("5.0.0"),
        version: str2u8("5.0.0"),
        machine: str2u8("RISC-V64"),
        domainname: str2u8("Rcore"),
    };
    let token = current_token().await;
    put_data(token, buf as *mut Utsname, uname)?;

    Ok(0)
}

pub async fn sys_clock_gettime(clock_id: usize, tp: usize) -> SyscallRet {
    trace!("[sys_clock_gettime]:clock_id:{},tp:usize:{}", clock_id, tp);
    if clock_id >= MAX_CLOCKS as usize {
        return Err(SysErrNo::EINVAL);
    }
    let token = current_token().await;
    put_data(token, tp as *mut UserTimeSpec,timer::get_usertime() )?;
    Ok(0)
}