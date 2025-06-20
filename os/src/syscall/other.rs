
use linux_raw_sys::general::MAX_CLOCKS;
use alloc::{string::String, vec};
use riscv::register::time;
use crate::{config::{MAX_KERNEL_RW_BUFFER_SIZE, TOTALMEM}, fs::{open_file, OpenFlags, NONE_MODE}, mm::{fill_str, get_target_ref, page_table::get_data, put_data, translated_byte_buffer, translated_refmut, translated_str}, syscall::flags::{Sysinfo, Utsname}, task::{current_process, current_task, current_token, sleeplist::sleep_until, task_count}, timer::{self, current_time, get_time_ms, get_usertime, usertime2_timeval, Tms, UserTimeSpec}, utils::error::{SysErrNo,  SyscallRet}};

pub async  fn sys_sysinfo(info: *const u8) -> SyscallRet {

   current_process().memory_set.lock().await.safe_put_data(
        info as *mut Sysinfo,
        Sysinfo::new(get_time_ms() / 1000, TOTALMEM, task_count()),
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

    let timer=timer::get_usertime();
    trace!("[sys_clock_gettime],timer:{:?}",timer);
  
   match clock_id {
    0 /* CLOCK_REALTIME */ => {
        let base = 0; 
        let mut ts = get_usertime();
        ts.tv_sec += base;
        current_process().memory_set.lock().await.
   safe_put_data(tp as *mut UserTimeSpec, ts).await?;
    }
    1 /* CLOCK_MONOTONIC */ => {
        let ts = get_usertime();
        current_process().memory_set.lock().await.
   safe_put_data(tp as *mut UserTimeSpec, ts).await?;
    }
    _ => return Err(SysErrNo::EINVAL),
}
    
    Ok(0)
}
pub async fn sys_nanosleep(req: *const UserTimeSpec, rem: *mut UserTimeSpec) -> SyscallRet {
info!(
        "[sys_nanosleep]: req:{:?},rem:{:?}",req,rem
    );
    let proc = current_process();
    let token = proc.get_user_token().await;
    if proc.manual_alloc_type_for_lazy(req).await.is_err() {
        return Err(SysErrNo::EFAULT);
    }

    // 2. 从用户空间读出 “请求的相对睡眠时间”
    let request_time: &UserTimeSpec = get_target_ref(token, req)?;
    // request_time 是一个 &UserTimeSpec，表示用户传来的 { tv_sec, tv_nsec }

    // 3. 计算“绝对”睡眠结束时刻 = 当前内核时间 + 相对时长
    let now = get_usertime();
    let deadline = now + (*request_time);

    // 4. 让出 CPU，直到“绝对时刻”到来或者被唤醒
    sleep_until(Some(usertime2_timeval(&deadline))).await;

    // 5. 睡醒后，检查是否真到 deadline，或者被信号打断
    let current_time = get_usertime();
    if current_time < deadline && !rem.is_null() {
        // 如果当前时间还没到 deadline，就说明被信号打断，需要把剩余时间写回 rem
        if proc.manual_alloc_type_for_lazy(rem).await.is_err() {
            return Err(SysErrNo::EFAULT);
        }
        // 计算剩余的纳秒数
        let remaining = deadline - current_time;
        let delta_nanos = remaining.as_nanos() as usize;
        let remaining_spec = UserTimeSpec {
            tv_sec:  (delta_nanos / 1_000_000_000) as usize,
            tv_nsec: (delta_nanos % 1_000_000_000) as usize,
        };
        // 将剩余时间写回用户空间
        put_data(token, rem, remaining_spec)?;
        // 被信号打断，返回 EINTR
        return Err(SysErrNo::ERESTART);
    }

    // 6. 如果 current_time >= deadline，就正常返回 0
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
    // if clock_id >= MAX_CLOCKS as usize {
    //     return Err(SysErrNo::EINVAL);
    // }
    // if flags != 0 {
    //     return Err(SysErrNo::EINVAL);
    // }
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
           
            return Err(SysErrNo::ERESTART);
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
    flags: usize,
) -> SyscallRet {

const AT_FDCWD: i32 = -100; // 代表当前工作目录
const AT_SYMLINK_NOFOLLOW: usize = 0x100; // 不跟随符号链接
const AT_EMPTY_PATH: usize = 0x1000; // 允许路径为空字符串

 const UTIME_NOW: usize = 0x3fffffff; // 设置为当前时间
 const UTIME_OMIT: usize = 0x3ffffffe; // 保持时间不变

 trace!(
    "[sys_utimensat] dirfd:{}, path:{:?}, times:{:?}, flags:{:#x}",
    dirfd, path, times, flags
);
if dirfd!=AT_FDCWD&&dirfd<0{
    return Err(SysErrNo::EBADF);
}

let pcb = current_process();
let token = pcb.memory_set.lock().await.token();

// 1. 从用户空间转换路径。NULL 指针被视为空字符串。
let path_str = if path.is_null() {
    String::new()
} else {
    translated_str(token, path)
};

// 2. 解析用户传入的时间参数。
let now_sec = (get_time_ms() / 1000) as u32;
let (mut atime_sec, mut mtime_sec) = (None, None);

if times.is_null() {
    // 如果 times 指针为 NULL，则访问和修改时间都设为当前时间。
    atime_sec = Some(now_sec);
    mtime_sec = Some(now_sec);
} else {
    // 从用户空间拷贝 timespec 结构体数据。
    let atime = get_data(token, times);
    let mtime = get_data(token, unsafe { times.add(1) });

    match atime.tv_nsec {
        UTIME_NOW => atime_sec = Some(now_sec), // 设置为当前时间
        UTIME_OMIT => (),                       // 保持不变
        _ => atime_sec = Some(atime.tv_sec as u32),
    };
    match mtime.tv_nsec {
        UTIME_NOW => mtime_sec = Some(now_sec),
        UTIME_OMIT => (),
        _ => mtime_sec = Some(mtime.tv_sec as u32),
    };
}

// 3. 根据路径和标志确定要操作的目标 INode。
let target_inode = if path_str.is_empty() {
    // --- 情况1：path 是 NULL 或空字符串 ---
    // 操作应该作用于 dirfd 本身。

    // 根据 man page，如果 path 是空字符串 ""，则必须设置 AT_EMPTY_PATH 标志。
    // 如果 path 是 NULL 指针，则不需要此标志。
    if !path.is_null() && (flags & AT_EMPTY_PATH) == 0 {
        return Err(SysErrNo::ENOENT); // 路径是 "" 但未设置 AT_EMPTY_PATH
    }

    // 在这种情况下，dirfd 必须是一个有效的文件描述符，不能是 AT_FDCWD。
    if dirfd == AT_FDCWD {
        return Err(SysErrNo::EBADF);
    }
    pcb.get_file(dirfd as usize ).await?.file()?
} else {
    // --- 情况2：path 是一个非空字符串 ---

    let follow_symlinks = (flags & AT_SYMLINK_NOFOLLOW) == 0;

    let abs_path = pcb.resolve_path_from_fd(dirfd, &path_str, follow_symlinks).await?;
    
    open_file(&abs_path, OpenFlags::O_RDONLY, 0o666)?.file()?
};

// 4. 在最终确定的 inode 上设置时间戳。
target_inode
    .set_timestamps(atime_sec, mtime_sec, None)
    .map(|_| 0) // 成功则返回 0
}

pub async  fn sys_sched_getaffinity(
    pid: i32,
    cpusetsize: usize,
    user_mask: *mut u8,
) -> SyscallRet {
    trace!(
        "[sys_sched_getaffinity] pid:{}, cpusetsize:{}, user_mask:{:?}",
        pid,
        cpusetsize,
        user_mask
    );

    if cpusetsize < 1 || cpusetsize > 1024 {
        return Err(SysErrNo::EINVAL);
    }

    if user_mask.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 获取当前任务的 CPU 掩码
    
    // 将掩码写入用户空间
    let token = current_token().await;
    current_process().manual_alloc_type_for_lazy(user_mask).await?;
    put_data(token, user_mask, 0)?;

    Ok(0)
}
pub  fn sys_sched_setaffinity(
    pid: i32,
    cpusetsize: usize,
    user_mask: *const u8,
) -> SyscallRet {
    trace!(
        "[sys_sched_setaffinity] pid:{}, cpusetsize:{}, user_mask:{:?}",
        pid,
        cpusetsize,
        user_mask
    );

    if cpusetsize < 1 || cpusetsize > 1024 {
        return Err(SysErrNo::EINVAL);
    }

    if user_mask.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    return Ok(0);
}