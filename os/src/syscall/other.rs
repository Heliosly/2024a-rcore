use core::mem::offset_of;

use linux_raw_sys::general::MAX_CLOCKS;
use alloc::vec;
use crate::{config::MAX_KERNEL_RW_BUFFER_SIZE, mm::{fill_str,  put_data, translated_byte_buffer}, syscall::flags::Utsname, task::{current_process, current_task, current_token}, timer::{self, current_time, UserTimeSpec}, utils::error::{SysErrNo,  SyscallRet}};



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

fn lcg(seed: &mut u64, buffer: &mut [u8]) {
    const A: u64 = 1664525;
    const C: u64 = 1013904223;
    const M: u64 = 1 << 32;

    for byte in buffer.iter_mut() {
        *seed = (*seed * A + C) % M;
        *byte = (*seed & 0xFF) as u8;
    }
}
fn xorshift64(seed: &mut u64, buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *byte = (*seed & 0xFF) as u8;
    }
}
pub async fn sys_getrandom(buf_ptr: *mut u8, len: usize, _flags: u32) -> SyscallRet {
    trace!("[sys_getrandom] buf_ptr: {:p}, len: {}, flags: {}", buf_ptr, len, _flags);

    if len == 0 {
        return Ok(0);
    }
    let token = current_token().await;
    if buf_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }
    let mut seed: u64 = (current_time().sec as u64) * 1_000_000 + current_time().usec as u64;
    seed ^= current_task().get_tid() as u64 + 1110111;
    let mut user_buffer = translated_byte_buffer(token, buf_ptr, len);
    if user_buffer.is_empty() {
        return Err(SysErrNo::EFAULT);
    }
    let mut written_bytes = 0;
    for chunk in user_buffer.iter_mut() {
        let chunk_len = chunk.len().min(len - written_bytes);
        let mut kernel_buffer = vec![0u8; chunk_len.min(MAX_KERNEL_RW_BUFFER_SIZE)];
        xorshift64(&mut seed, &mut kernel_buffer);
        chunk[..kernel_buffer.len()].copy_from_slice(&kernel_buffer);
        written_bytes += kernel_buffer.len();
        if written_bytes >= len {
            break;
        }
    }
    Ok(written_bytes)
}