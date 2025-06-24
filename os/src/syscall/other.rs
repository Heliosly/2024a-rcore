
use linux_raw_sys::general::{CLOCK_MONOTONIC, CLOCK_REALTIME, MAX_CLOCKS};
use alloc::{string::String, vec};
use riscv::register::time;
use crate::{config::{MAX_KERNEL_RW_BUFFER_SIZE, TOTALMEM}, fs::{open_file, OpenFlags, NONE_MODE}, mm::{fill_str, get_target_ref, page_table::get_data, put_data, translated_byte_buffer, translated_refmut, translated_str, UserBuffer}, syscall::flags::{Sysinfo, Utsname}, task::{current_process, current_task, current_token, sleeplist::sleep_until, task_count, PID2PC}, timer::{self, current_time, get_time_ms, get_usertime, usertime2_timeval, TimeVal, Tms, UserTimeSpec}, utils::error::{SysErrNo,  SyscallRet}};

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

pub async fn sys_sched_getaffinity(
    pid: i32,
    cpusetsize: usize,
    user_mask: *mut usize,
) -> SyscallRet {
    trace!(
        "[sys_sched_getaffinity] pid:{}, cpusetsize:{}, user_mask:{:?}",
        pid,
        cpusetsize,
        user_mask
    );
    
    // --- 核心修正 ---
    // 1. 定义内核实际支持的 CPU 数量和掩码大小
    const KERNEL_NR_CPUS: usize = 1; // 我们的系统是单核
    const KERNEL_CPU_MASK_SIZE: usize = (KERNEL_NR_CPUS + 7) / 8; // 计算所需字节数，这里是 1

    // 2. 检查用户提供的 cpusetsize 是否足够
    //    如果用户提供的缓冲区太小，我们什么都不做，直接返回我们需要的真实大小。
    //    这是 man page 规定的行为。
    if cpusetsize < KERNEL_CPU_MASK_SIZE {
        return Err(SysErrNo::EINVAL);
    }

    // 3. 获取目标任务的亲和性掩码
    //    （在单核模型中，我们忽略 pid，因为所有任务都一样）
    //    TODO: 如果需要支持多进程，这里需要根据 pid 查找任务
    let current_pcb = current_process(); // 假设有 current_process()
    let affinity_mask :usize= 1;
    
    // 4. 安全地将掩码写入用户空间
    let token = current_token().await;
    // 确保用户指针有效
    current_pcb.manual_alloc_type_for_lazy(user_mask).await?;

    *translated_refmut(token, user_mask)?=affinity_mask;


    // 7. 返回内核实际支持的掩码大小
    Ok(KERNEL_CPU_MASK_SIZE) 
}
pub async fn sys_sched_setaffinity(
    pid: i32,
    cpusetsize: usize,
    user_mask_ptr: *const usize,
) -> SyscallRet {
    trace!(
        "[sys_sched_setaffinity] pid:{}, cpusetsize:{}, user_mask:{:?}",
        pid,
        cpusetsize,
        user_mask_ptr
    );

   //多核心todo

    // 成功
    Ok(0)
}





// Scheduling policies
pub const SCHED_OTHER: i32 = 0;
pub const SCHED_FIFO: i32 = 1;
pub const SCHED_RR: i32 = 2;
// ... 其他策略 (BATCH, IDLE, DEADLINE)

/// C-compatible struct sched_param
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SchedParam {
    pub sched_priority: i32,
}
/// man 2: int sched_setscheduler(pid_t pid, int policy, const struct sched_param *param);
pub async fn sys_sched_setscheduler(
    pid: i32,
    policy: i32,
    param_ptr: *const SchedParam,
) -> SyscallRet {
    trace!(
        "[sys_sched_setscheduler] pid:{}, policy:{}, param_ptr:{:?}",
        pid,
        policy,
        param_ptr
    );
    
    // --- 参数验证 ---

    // 1. 验证 pid
    if pid < 0&&!PID2PC.lock().contains_key(&(pid as usize))  {
        return Err(SysErrNo::EINVAL);
    }

    // 2. 验证用户提供的指针
    if param_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }
    
    // 3. 验证策略 (Policy)
    //    由于我们只实现了 CFS (SCHED_OTHER)，我们只接受这个策略。
    //    尝试设置任何其他策略都是无效的。
    // if policy != SCHED_OTHER {
    //     // 对于实时策略，通常需要 root 权限，可以返回 EPERM。
    //     // 对于其他无效策略，返回 EINVAL。我们统一返回 EINVAL。
    //     return Err(SysErrNo::EINVAL);
    // }
    // --- 应用设置 ---
    

    // e.g., current_pcb.set_scheduler(policy, param);

    // 成功
    Ok(0)
}

/// man 2: int sched_getscheduler(pid_t pid);
pub  fn sys_sched_getscheduler(pid: i32) -> SyscallRet {
    trace!("[sys_sched_getscheduler] pid:{}", pid);
    
    
    
    if pid < 0&&!PID2PC.lock().contains_key(&(pid as usize))  {
        return Err(SysErrNo::EINVAL);
    }

    // 在我们的模型中，调度器是 CFS，对应的策略是 SCHED_OTHER。
    // 这个值是固定的，因为我们不支持动态改变为实时策略。
    let policy = SCHED_OTHER;

    // 成功，返回策略值
    Ok(policy as usize)
}


/// man 2: int sched_getparam(pid_t pid, struct sched_param *param);
pub async fn sys_sched_getparam(pid: i32, param_ptr: *mut SchedParam) -> SyscallRet {
    trace!(
        "[sys_sched_getparam] pid:{}, param_ptr:{:?}",
        pid,
        param_ptr
    );

    // 1. 验证 pid 和指针，遵循您的风格
    if pid < 0 && !PID2PC.lock().contains_key(&(pid as usize))  {
        return Err(SysErrNo::EINVAL);
    }
    if param_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }

    // 2. 准备要返回的参数
    //    在我们的 CFS-only 模型中，任何进程的 sched_priority 都是 0。
    let param = SchedParam { sched_priority: 0 };

    let token = current_token().await;
    let current_pcb = current_process();

    // 确保用户指针指向的内存已映射
    current_pcb.manual_alloc_type_for_lazy(param_ptr).await?;
    
    // 翻译用户地址
    *translated_refmut(token, param_ptr)?=param;
   

    // 成功
    Ok(0)
}

/// man 2: int sched_setparam(pid_t pid, const struct sched_param *param);
pub  fn sys_sched_setparam(pid: i32, param_ptr: *const SchedParam) -> SyscallRet {
    trace!(
        "[sys_sched_setparam] pid:{}, param_ptr:{:?}",
        pid,
        param_ptr
    );
    

    // 1. 验证 pid
    if pid < 0 && !PID2PC.lock().contains_key(&(pid as usize))  {
        return Err(SysErrNo::EINVAL);
    }

    // 2. 验证用户提供的指针
    if param_ptr.is_null() {
        return Err(SysErrNo::EFAULT);
    }
    

    
    // 成功
    Ok(0)
}
/// man 2: int clock_getres(clockid_t clockid, struct timespec *res);
pub async fn sys_clock_getres(clockid: u32, res_ptr: *mut UserTimeSpec) -> SyscallRet {
    trace!(
        "[sys_clock_getres] clockid:{}, res_ptr:{:?}",
        clockid,
        res_ptr
    );

    // 1. 验证 clockid
    //    我们只支持最常见的几种时钟
    match clockid {
        CLOCK_REALTIME | CLOCK_MONOTONIC => {
            // 这两种是我们支持的，继续执行
        }
        _ => {
            // 其他 clockid 我们不支持
            return Err(SysErrNo::EINVAL);
        }
    }

    // 2. 验证用户指针
    //    如果 res_ptr 为 NULL，我们什么都不做，直接成功返回 0。这是 man page 规定的行为。
    if res_ptr.is_null() {
        return Ok(0);
    }

    // 3. 准备要返回的分辨率
    //    我们声明系统的时钟分辨率为 1 纳秒。这是一个常见且安全的值。
    let resolution = UserTimeSpec {
        tv_sec: 0,
        tv_nsec: 1,
    };

    let token = current_token().await;
    let current_pcb = current_process();

   
    current_pcb.manual_alloc_type_for_lazy(res_ptr).await?;

   *translated_refmut(token, res_ptr)? = resolution;

    // 成功
    Ok(0)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ITimerVal {
    pub it_interval: TimeVal, // 周期性定时器的间隔
    pub it_value: TimeVal,    // 下一次触发的剩余时间
}
pub const ITIMER_REAL: i32 = 0;
pub const ITIMER_VIRTUAL: i32 = 1;
pub const ITIMER_PROF: i32 = 2;
/// 获取一个间隔定时器的值
pub async fn sys_getitimer(which: i32, value_ptr: *mut ITimerVal) -> SyscallRet {
    if !(ITIMER_REAL..=ITIMER_PROF).contains(&which) {
        return Err(SysErrNo::EINVAL);
    }

    let pcb = current_process();
    let token = pcb.get_user_token().await;
    pcb.manual_alloc_type_for_lazy(value_ptr).await?;
    // 锁定对应的定时器
    let kernel_timer = pcb.timers[which as usize].lock().await;

    // 将内核格式（纳秒）转换为用户空间格式（ITimerVal）
    let user_itimerval = ITimerVal {
        it_interval: TimeVal::from_ns(kernel_timer.interval),
        it_value: TimeVal::from_ns(kernel_timer.value),
    };

    // 拷贝到用户空间
    *translated_refmut(token,value_ptr)?=user_itimerval;
   Ok(0)
}

/// 设置一个间隔定时器的值
pub async fn sys_setitimer(
    which: i32,
    new_value_ptr: *const ITimerVal,
    old_value_ptr: *mut ITimerVal,
) -> SyscallRet {
    if !(ITIMER_REAL..=ITIMER_PROF).contains(&which) {
        return Err(SysErrNo::EINVAL);
    }

    let process = current_process();
    let which_idx = which as usize;
   let token = process.get_user_token().await;
    process.manual_alloc_type_for_lazy(new_value_ptr).await?;
    // 1. 如果需要，先获取并返回旧值
    if !old_value_ptr.is_null() {

    process.manual_alloc_type_for_lazy(old_value_ptr).await?;
        let old_kernel_timer = process.timers[which_idx].lock().await;
        let old_user_itimerval = ITimerVal {
            it_interval: TimeVal::from_ns(old_kernel_timer.interval),
            it_value: TimeVal::from_ns(old_kernel_timer.value),
        };
        *translated_refmut(token,old_value_ptr)?= old_user_itimerval;
    }

    // 2. 从用户空间获取新值
    let  new_user_itimerval =   *get_target_ref(token, new_value_ptr)?;
      

    // 3. 更新内核中的定时器
    let mut kernel_timer = process.timers[which_idx].lock().await;
    kernel_timer.value = u64::from(new_user_itimerval.it_value);
    kernel_timer.interval = u64::from(new_user_itimerval.it_interval);

    // **** 关键：如果设置的是 ITIMER_REAL，需要与全局定时器后端交互 ****
    // 我们暂时只更新 TCB 中的值，后端逻辑在第 3 步实现
    // 例如，如果是一个新的真实定时器，需要将它添加到全局的定时器队列中
    if which == ITIMER_REAL {
        crate::timer::set_real_timer(process.get_pid(), kernel_timer.value).await;
    }

   Ok(0)
}
pub fn sys_umaske()->SyscallRet{
    warn!("[umask]");
    Ok(0)
}