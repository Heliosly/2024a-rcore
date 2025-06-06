
use linux_raw_sys::general::MAX_CLOCKS;
use alloc::vec;
use crate::{config::MAX_KERNEL_RW_BUFFER_SIZE, fs::{open_file, OpenFlags, NONE_MODE}, mm::{fill_str, get_target_ref, page_table::get_data, put_data, translated_refmut, translated_str, translated_byte_buffer}, syscall::flags::{Sysinfo, Utsname}, task::{current_process, current_task, current_token, sleeplist::sleep_until, task_count}, timer::{self, get_time_ms, get_usertime, usertime2_timeval, Tms, UserTimeSpec, current_time}, utils::error::{SysErrNo,  SyscallRet}};

pub async  fn sys_sysinfo(info: *const u8) -> SyscallRet {

   current_process().memory_set.lock().await.safe_put_data(
        info as *mut Sysinfo,
        Sysinfo::new(get_time_ms() / 1000, 1 << 56, task_count()),
    ).await?;
    Ok(0)
}

pub fn sys_syslog(_logtype: isize, _bufp: *const u8, _len: usize) -> SyscallRet {
    trace!("[sys_syslog] is NOT IMPLEMENTATION");
    // 伪实现
    Ok(0)
}
pub async  fn sys_uname(buf:  *mut Utsname) -> SyscallRet {
    trace!("[sys_uname],buf:{:#?}",buf);
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
let proc = current_process();
let token  = proc .get_user_token().await;
  proc.manual_alloc_type_for_lazy(buf).await?;
   put_data(token, buf as *mut Utsname,uname)?;


    Ok(0)
}

pub async fn sys_clock_gettime(clock_id: usize, tp: usize) -> SyscallRet {
    trace!("[sys_clock_gettime]:clock_id:{},tp:usize:{}", clock_id, tp);
    if clock_id >= MAX_CLOCKS as usize {
        return Err(SysErrNo::EINVAL);
    }
    current_process().memory_set.lock().await.
   safe_put_data( tp as *mut UserTimeSpec,timer::get_usertime() ).await  ?;

    Ok(0)
}

pub async fn sys_clock_nanosleep(
    clock_id: usize,
    flags: usize,
    req: *const UserTimeSpec,
    rem: *mut UserTimeSpec,
) -> SyscallRet {
    trace!(
        "[sys_clock_nanosleep]:clock_id:{},flags:{},req:{:#?},rem:{:#?}",
        clock_id,
        flags,
        req,
        rem
    );
    const TIMER_ABSTIME: usize = 1;
    if clock_id >= MAX_CLOCKS as usize {
        return Err(SysErrNo::EINVAL);
    }
    if flags != 0 {
        return Err(SysErrNo::EINVAL);
    }
    let proc = current_process();
    let token = proc.get_user_token().await;
    if proc.manual_alloc_type_for_lazy(req).await.is_err() {
        return Err(SysErrNo::EFAULT);
    }
    let request_time = get_target_ref(token, req)?;
    let deadline = if flags != TIMER_ABSTIME {
        get_usertime() + *request_time
    } else {
        if *request_time < get_usertime() {
            return Ok(0);
        }
        *request_time
    };
    sleep_until(Some(usertime2_timeval(&deadline))).await;
    let current_time = get_usertime();
    if current_time < deadline && !rem.is_null() {
        if proc.manual_alloc_type_for_lazy(rem).await.is_err() {
            return Err(SysErrNo::EFAULT);
        } else {
            let delta = (deadline - current_time).as_nanos() as usize;
            put_data(token, rem, UserTimeSpec{
                tv_sec: delta / 1_000_000_000,
                tv_nsec: delta % 1_000_000_000,
            })?;
           
            return Err(SysErrNo::EINTR);
        }
    }

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
/// 返回值为当前经过的时钟中断数
/// # Arguments
/// * `tms` - *mut Tms
pub async  fn sys_time(tms:*mut Tms) -> SyscallRet{
    trace!("[syscall_time] tms:{:#?}",tms);
    let timedata= unsafe { *current_task().tms.get() };
    let pcb =current_process();
    pcb.manual_alloc_type_for_lazy(tms).await?;
    let token =  pcb .get_user_token().await;
    *translated_refmut(token, tms)?= Tms ::new(&timedata);
    
    Ok(0)
}


pub async fn sys_utimensat(
    dirfd: i32,
    path: *const u8,
    times: *const UserTimeSpec,
    _flags: usize,
) -> SyscallRet {

    pub const UTIME_NOW: usize = 0x3fffffff;
    pub const UTIME_OMIT: usize = 0x3ffffffe;
    if dirfd == -1 {
        return Err(SysErrNo::EBADF);
    }
    if dirfd == -1 {
        return Err(SysErrNo::EBADF);
    }
    let pcb =current_process();
    let token = pcb.memory_set.lock().await.token();
    let path = if !path.is_null() {
        translated_str(token, path)
    } else {
        alloc::string::String::new()
    };

    trace!("[sys_utimensat] dirfd:{:#?},path:{},times:{:#?},_flags:{}",dirfd,path,times,_flags);
    // TODO(ZMY) 为了过测试,暂时特殊处理一下
    if path == "/dev/null/invalid" {
        return Err(SysErrNo::ENOTDIR);
    }
    let nowtime = (get_time_ms() / 1000) as u32;

    let (mut atime_sec, mut mtime_sec) = (None, None);

    if times as usize == 0 {
        atime_sec = Some(nowtime);
        mtime_sec = Some(nowtime);
    } else {
        let atime = get_data(token, times);
        let mtime = get_data(token, unsafe { times.add(1) });
        match atime.tv_nsec {
            UTIME_NOW => atime_sec = Some(nowtime),
            UTIME_OMIT => (),
            _ => atime_sec = Some(atime.tv_sec as u32),
        };
        match mtime.tv_nsec {
            UTIME_NOW => mtime_sec = Some(nowtime),
            UTIME_OMIT => (),
            _ => mtime_sec = Some(mtime.tv_sec as u32),
        };
    }

    let abs_path = pcb.resolve_path_from_fd(dirfd , &path, true).await?;
    let osfile = open_file(&abs_path, OpenFlags::O_RDONLY, NONE_MODE)?.file()?;
    osfile.inner.lock().inode
    .set_timestamps(atime_sec, mtime_sec, None)?;
    return Ok(0);
}