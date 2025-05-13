use crate::{mm::translated_refmut, task::current_token};

use super::flags::UtsName;

fn fill_str(buf: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&bytes[..len]);
    buf[len] = 0; // null-terminator
}
const SYSNAME: &str = "rCore";
const NODENAME: &str = "rcore-host";
const RELEASE: &str = "0.1";
const VERSION: &str = "0.1 (rcore)";
const MACHINE: &str = "riscv64";
pub fn sys_uname(buf: usize) -> isize {
    let token = current_token();
    let uts: &mut UtsName = translated_refmut(token, buf as *mut  UtsName);
    fill_str(&mut uts.sysname, SYSNAME);
    fill_str(&mut uts.nodename, NODENAME);
    fill_str(&mut uts.release, RELEASE);
    fill_str(&mut uts.version, VERSION);
    fill_str(&mut uts.machine, MACHINE);
    0
}