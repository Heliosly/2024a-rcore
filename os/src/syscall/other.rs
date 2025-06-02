
use linux_raw_sys::general::MAX_CLOCKS;

use crate::{mm::{fill_str, get_target_ref, put_data, translated_refmut}, syscall::flags::Utsname, task::{current_process, current_task, current_token, sleeplist::sleep_until}, timer::{self, get_usertime, usertime2_timeval, Tms, UserTimeSpec}, utils::error::{SysErrNo,  SyscallRet}};




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

/// 返回值为当前经过的时钟中断数
/// # Arguments
/// * `tms` - *mut Tms
pub async  fn syscall_time(tms:*mut Tms) -> SyscallRet{
    trace!("[syscall_time] tms:{:#?}",tms);
    let timedata= unsafe { *current_task().tms.get() };
    let pcb =current_process();
    pcb.manual_alloc_type_for_lazy(tms).await?;
    let token =  pcb .get_user_token().await;
    *translated_refmut(token, tms)?= Tms ::new(&timedata);
    
    Ok(0)
}
