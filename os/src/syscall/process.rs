//! Process management syscalls
//!


use alloc::{
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::{
    config::{MAX_SYSCALL_NUM, MMAP_TOP, PAGE_SIZE},
    fs::{open_file, OpenFlags, NONE_MODE},
    mm::{
        flush_tlb, get_target_ref, page_table::copy_to_user_bytes, translated_byte_buffer, translated_refmut, translated_str, MapArea, MapAreaType, MapPermission, MapType, TranslateRefError, VirtAddr, VirtPageNum
    },
    syscall::flags::{MmapFlags, MmapProt},
    task::{
        current_process, current_task, current_token, exit_current_and_run_next, set_priority,
        yield_now, CloneFlags, TaskStatus, PID2PC,
    },
    timer::{ get_time_us, TimeVal},
    utils::{
        error::{SysErrNo, SyscallRet},
        page_round_up,
        string::get_abs_path,
    },
};

/// Task information
#[repr(C)]
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

pub fn sys_getppid() -> SyscallRet {
    trace!("[sys_getppid] pid :{}", current_task().get_pid());
    Ok(current_process().parent())
}
pub async  fn sys_exit(exit_code: i32) -> SyscallRet {
    trace!("kernel:pid[{}] sys_exit", current_task().get_pid());

    exit_current_and_run_next(exit_code).await;
    Ok(exit_code as usize)
}

pub async fn sys_yield() -> SyscallRet {
    trace!("kernel: sys_yield");
    yield_now().await;
    Ok(0)
}

pub fn sys_getpid() -> SyscallRet {
    trace!("kernel: sys_getpid pid:{}", current_task().get_pid());
    Ok(current_task().get_pid())
}
/// # Arguments for riscv
/// * `flags` - usize
/// * `user_stack` - usize
/// * `ptid` - usize
/// * `tls` - usize
/// * `ctid` - usize
pub async fn sys_clone(args: [usize; 6]) -> SyscallRet {
    //解析参数
    let flags = args[0];
    let user_stack = args[1];
    let ptid = args[2];
    let tls = args[3];
    let ctid = args[4];
    let proc = current_process();

    let flags =
        CloneFlags::from_bits(flags & !0x3f).expect(&format!("unsupport cloneflags : {}", flags));
    debug!(
        "[sys_clone] flags:{:#?},user_stack:{:#x},ptid:{:#x},tls:{:#x},ctid:{:#x}",
        flags, user_stack, ptid, tls, ctid
    );
    // Invalid combination checks for clone flags
    // if flags.contains(CloneFlags::CLONE_SIGHAND) && !flags.contains(CloneFlags::CLONE_VM) {
    //     // CLONE_SIGHAND requires CLONE_VM
    //     return Err(SysErrNo::EINVAL);
    // }

    if flags.contains(CloneFlags::CLONE_SETTLS) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_SETTLS requires CLONE_VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_THREAD) && !flags.contains(CloneFlags::CLONE_SIGHAND) {
        // CLONE_THREAD requires CLONE_SIGHAND (to share signal handlers)
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_THREAD) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_THREAD requires CLONE_VM (threads must share address space)
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_CHILD_SETTID) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_CHILD_SETTID only makes sense when sharing VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_CHILD_CLEARTID only makes sense when sharing VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_PARENT_SETTID) && !flags.contains(CloneFlags::CLONE_VM) {
        // CLONE_PARENT_SETTID only makes sense when sharing VM
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_PIDFD) && !flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
        // CLONE_PIDFD requires CLONE_PARENT_SETTID
        return Err(SysErrNo::EINVAL);
    }

    if flags.contains(CloneFlags::CLONE_VFORK) && flags.contains(CloneFlags::CLONE_THREAD) {
        // VFORK and THREAD are mutually exclusive
        return Err(SysErrNo::EINVAL);
    }

    proc.clone_task(flags, user_stack, ptid, tls, ctid).await
}
// pub fn sys_fork() -> isize {
//     trace!("kernel:pid[{}] sys_fork", current_task().get_pid());
//     let current_process = current_process();

//     let new_pro = current_process.fork();
//     let new_pid = new_pro.get_pid();
//     // debug!("new_pid:{}",new_pid);

//     // modify trap context of new_task, because it returns immediately after switching
//     // we do not have to move to next instruction since we have done it before
//     // for child process, fork returns 0
//     // add new task to scheduler
//     // let inner = new_task.inner_exclusive_access();
//     // for area in inner.memory_set.areas.iter(){
//     //     for (_,ppn) in area.data_frames.iter(){
//     //         if ppn.ppn().0==0x81901 {
//     //             print!("\nfork in pid{}\n",new_pid);
//     //             println!("in fork pid =3 ,ppn={:#x},frametracker_ptr:{:#x}",
//     //             ppn.ppn().0,ppn as *const _ as usize);
//     //         }
//     //         if new_task.get_pid()==3 {
//     //             println!("in fork pid =3 ,ppn={:#x},frametracker_ptr:{:#x}",
//     //             ppn.ppn().0,ppn as *const _ as usize);
//     //         }
//     //     }
//     // }
//     // drop(inner);

//     new_pid as isize
// }

/// 参考 https://man7.org/linux/man-pages/man2/execve.2.html
pub  async  fn sys_execve(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_exec", current_task().get_pid());
    let process = current_process();

    let token = process.get_user_token().await;
    let mut path = translated_str(token, path);
    //处理argv参数
    let mut argv_vec = Vec::<String>::new();
    // if !argv.is_null() {
    //     let argv_ptr = *translated_ref(token, argv);
    //     if argv_ptr != 0 {
    //         argv_vec.push(path.clone());
    //         unsafe {
    //             argv = argv.add(1);
    //         }
    //     }
    // }
    loop {
        if argv.is_null() {
            break;
        }
        let argv_ptr = *get_target_ref(token, argv)?;
        if argv_ptr == 0 {
            break;
        }
        argv_vec.push(translated_str(token, argv_ptr as *const u8));
        unsafe {
            argv = argv.add(1);
        }
    }
    if path.ends_with(".sh") {
        //.sh文件不是可执行文件，需要用busybox的sh来启动
        argv_vec.insert(0, String::from("sh"));
        argv_vec.insert(0, String::from("busybox"));
        path = String::from("/busybox");
    }

    // if path.ends_with("ls") || path.ends_with("xargs") || path.ends_with("sleep") {
    //     //ls,xargs,sleep文件为busybox调用，需要用busybox来启动
    //     argv_vec.insert(0, String::from("busybox"));
    //     path = String::from("/busybox");
    // }

    debug!("[sys_execve] path is {},arg is {:?}", path, argv_vec);
    let mut env = Vec::<String>::new();
    loop {
        if envp.is_null() {
            break;
        }
        let envp_ptr = *get_target_ref(token, envp)?;
        if envp_ptr == 0 {
            break;
        }
        env.push(translated_str(token, envp_ptr as *const u8));
        unsafe {
            envp = envp.add(1);
        }
    }
    let env_path = "PATH=/:/bin:".to_string();
    if !env.contains(&env_path) {
        env.push(env_path);
    }

    let env_ld_library_path = "LD_LIBRARY_PATH=/lib:/lib/glibc:/lib/musl:".to_string();
    if !env.contains(&env_ld_library_path) {
        env.push(env_ld_library_path);
    }

    let env_enough = "ENOUGH=100000".to_string();
    if !env.contains(&env_enough) {
        //设置系统最大负载
        env.push(env_enough);
    }

    debug!("[sys_execve] env is {:?}", env);

    let cwd = if !path.starts_with('/') {
        let cwd_lock = process.cwd.lock().await;
        cwd_lock.clone()
    } else {
        "/".to_string()
    };
    let abs_path = get_abs_path(&cwd, &path);
    let app_inode = open_file(&abs_path, OpenFlags::O_RDONLY, NONE_MODE)?;

    let elf_data = app_inode.file()?.read_all();
    process.exec(&elf_data, &argv_vec, &mut env).await?;
    process.memory_set.lock().await.activate();
    Ok(argv_vec.len())
}

pub fn sys_settidaddress() -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_settidaddress NOT IMPLEMENTED",
        current_task().get_pid()
    );
    //todo(Heliosly)
    Ok(current_task().id.as_usize())
}
pub fn sys_getuid() -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_getuid NOT IMPLEMENTED",
        current_task().get_pid()
    );
    //todo(heliosly)
    Ok(0)
}
/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub async  fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> SyscallRet {
    trace!("kernel: sys_waitpid");
    let proc = current_process();
    // find a child process

    // ---- access current PCB exclusively

    // for i in inner.children.iter(){
    //     print!(" {}",{i.get_pid()});
    // }

    // println!(" ");

    let mut childvec = proc.children.lock().await;
    if !childvec
        .iter()
        .any(|p| pid == -1 || pid as usize == p.get_pid())
    {
        debug!(
            "cant find pid:{} in parent pid:{},and children count = : {} ",
            pid,
            proc.get_pid(),
            childvec.len()
        );
        return Err(SysErrNo::ECHILD);
        // ---- release current PCB
    }
    let mut pair = None;
    for (idx, p) in childvec.iter().enumerate() {
        if p.is_zombie().await && (pid == -1 || pid as usize == p.get_pid()) {
            pair = Some((idx, p));
            break;
        }
    }
    if let Some((idx, child_task)) = pair {
        debug!("chiled idx is removed");

        PID2PC.lock().remove(&child_task.get_pid());
        let child = childvec.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        let found_pid = child.get_pid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.exit_code();

        // ++++ release child PCB
        *translated_refmut(proc.get_user_token().await, exit_code_ptr)? = exit_code;
        assert_eq!(
            Arc::strong_count(&child),
            1,
            "strong_count is incorrect,{}",
            Arc::strong_count(&child)
        );
        Ok(found_pid)
    } else {
        return Err(SysErrNo::EAGAIN);
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub async fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
        current_task().get_pid()
    );
    let usec = get_time_us().to_ne_bytes();

    let sec = (get_time_us() / 1000000).to_ne_bytes();

    let token = current_token().await;

    let bufs = translated_byte_buffer(token, _ts as *const u8, 16);
    let mut i = 0;
    for buf in bufs {
        for atm in buf {
            if i >= 8 {
                *atm = usec[i - 8];
            } else {
                *atm = sec[i];
            }

            i += 1;
        }
    }
    Ok(0)
}
// pub fn sys_gettimeofday(tp: usize, tz: usize) -> isize {
//     if tp == 0 {
//         return -1;
//     }
//     let time_ms = get_time_ms(); // 你要实现这个函数，返回毫秒时间戳

//     // 转成 timeval 结构体
//     let seconds = time_ms / 1000;
//     let micros = (time_ms % 1000) * 1000;

//     let timeval = TimeVal { sec: seconds , usec: micros  };
//     // 写入用户空间\

//     *translated_refmut(current_token(), tp as *mut TimeVal) = timeval;
//     0
// }

pub async  fn sys_exitgroup(exit_code: i32) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_exit_exitgroup NOT IMPLEMENTED",
        current_task().get_pid()
    );
    exit_current_and_run_next(exit_code).await;
    Ok(0)
}
/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> SyscallRet {
    trace!("kernel:pid[{}] sys_task_info ", current_task().get_pid());
    Ok(0)
}

// /// YOUR JOB: Implement mmap.
// pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
//     trace!(
//         "kernel:pid[{}] sys_mmap addr={:#x}, len={:#x}  ",
//         current_task().get_pid(), _start, _len
//     );
//     if _start % PAGE_SIZE != 0 || _port & !0x7 != 0 || _port & 0x7 == 0 {
//         return -1;
//     }
//     let arc = current_process();
//     bitflags! {
//         /// map permission corresponding to that in pte: `R W X U`
//         pub struct Portpomiss: u8 {
//             ///Readable
//             const R = 1 << 0;
//             ///Writable
//             const W = 1 << 1;
//             ///Excutable
//             const X = 1 << 2;

//         }
//     }
//     let portpomis = Portpomiss::from_bits_truncate(_port as u8);
//     let mut flag: MapPermission = MapPermission::empty();
//     flag |= MapPermission::U;
//     if portpomis.contains(Portpomiss::R) {
//         flag |= MapPermission::R;
//     }
//     if portpomis.contains(Portpomiss::W) {
//         flag |= MapPermission::W;
//     }
//     if portpomis.contains(Portpomiss::X) {
//         flag |= MapPermission::X;
//     }
//     let _end = _start + _len;

//     let end: VirtAddr = _end.into();
//     let start: VirtAddr = _start.into();
//     if arc
//         .memory_set
//         .lock()
//         .insert_framed_area_peek_for_mmap(start, end, flag)
//     {
//         -1
//     } else {
//         4
//     }
// }
/// 参考 https://man7.org/linux/man-pages/man2/mmap.2.html
pub async fn sys_mmap(
    addr: usize,
    len: usize,
    prot: u32,
    flags: u32,
    fd: usize,
    off: usize,
) -> SyscallRet {
    //需要有严格的判断返回错误的顺序！！！
    // 0. flags 不能全 0
    if flags == 0 {
        return Err(SysErrNo::EINVAL);
    }
    let flags = match MmapFlags::from_bits(flags) {
        Some(f) => f,
        None => return Err(SysErrNo::EINVAL),
    };

    // 1. 长度不能为 0
    if len == 0 {
        return Err(SysErrNo::EINVAL);
    }
    // 2. 偏移量必须页对齐
    if off % PAGE_SIZE != 0 {
        return Err(SysErrNo::EINVAL);
    }

    // 3. 匿名映射 | 文件映射：fd 检查
    let anon = flags.contains(MmapFlags::MAP_ANONYMOUS);
    if !anon {
        // 非匿名必须提供合法 fd
        if fd == usize::MAX {
            return Err(SysErrNo::EBADE);
        }
    }
    // 4. prot -> MapPermission
    let mmap_prot = match MmapProt::from_bits(prot) {
        Some(f) => f,
        None => return Err(SysErrNo::EINVAL),
    };
    let map_perm: MapPermission = mmap_prot.into();
    debug!(
        "[sys_mmap]: addr {:#x}, len {:#x}, fd {}, offset {:#x}, flags {:?}, prot is {:?}, map_perm {:?}",
        addr, len, fd as isize, off, flags,mmap_prot, map_perm
    );
    // 5. MAP_FIXED 且 addr == 0 禁止
    if flags.contains(MmapFlags::MAP_FIXED) && addr == 0 {
        return Err(SysErrNo::EPERM);
    }

    // 6. 计算页对齐后的长度和页数
    let len = page_round_up(len);
    let pages = len / PAGE_SIZE;

    let proc = current_process();
    let mut ms = proc.memory_set.lock().await;

    let fd_table = proc.fd_table.lock().await;
    // ——————————————————————————————————————————
    // 7. 如果是文件映射，要检查文件权限和 off_file

    let _file = if !anon {
        // 7.1 拿到文件对象
        let file = match fd_table.get(fd) {
            Some(f) => f.clone().unwrap(),
            None => return Err(SysErrNo::EBADF),
        };
        // 7.2 写映射需可写权限
        if map_perm.contains(MapPermission::W) && !file.writable()? {
            return Err(SysErrNo::EACCES);
        }
        // 7.3 offset 超出文件长度？
        if off > file.fstat().st_size as usize {
            return Err(SysErrNo::EINVAL);
        }

        unimplemented!();
        Some(file.clone())
    } else {
        None
    };

    let va = VirtAddr::from(addr);
    let vpn = va.floor();
    let end_vpn = VirtPageNum::from(vpn.0 + pages);
    let range = core::ops::Range {
        start: (vpn),
        end: (end_vpn),
    };
    // ——————————————————————————————————————————
    // 8. 按 MAP_FIXED / MAP_FIXED_NOREPLACE / hint / 自动选择地址
    let base = if flags.contains(MmapFlags::MAP_FIXED) {
        // 8.1 强制定位：先 munmap 冲突区

        if ms.areatree.is_overlap(&range) {
            ms.munmap(vpn, end_vpn);
        }

        va
    } else if flags.contains(MmapFlags::MAP_FIXED_NOREPLACE) {
        // 8.2 不替换：如果冲突则失败
        if ms.areatree.is_overlap(&range) {
            return Err(SysErrNo::EEXIST);
        }
        va
    } else {
        // 8.3 默认，从 addr 附近或全局搜索
        match ms.areatree.alloc_pages(pages) {
            Some(vpn) => VirtAddr::from(vpn),
            None => {
                warn!(
                    "[sys_mmap] no available gap for mapping {} pages from {:?}",
                    pages, vpn
                );
                return Err(SysErrNo::ENOMEM);
            }
        }
    };
    if base.0 >= MMAP_TOP {
        return Err(SysErrNo::ENOMEM);
    }
    // ——————————————————————————————————————————
    // 9. 构造 VMA / MapArea
    let mut area = MapArea::new(
        base,
        VirtAddr::from(base.0 + len),
        MapType::Framed,
        map_perm,
        MapAreaType::Mmap, // Option<Arc<File>>
    );

    debug!("[sys_mmap]mmap ok,base:{:#x}", base.0);

    // 10. 特殊 flags 的额外处理
    if flags.contains(MmapFlags::MAP_POPULATE) {
        // 立即为每页缺页、填充物理页
        area.map(&mut ms.page_table);
    }
    // if flags.contains(MmapFlags::MAP_LOCKED) {
    //     // mlock：锁定这些物理页
    //     area.lock(&mut ms.page_table)?;
    // }todo(heliosly)
    // if flags.contains(MmapFlags::MAP_STACK) {
    //     // 设置向下扩展属性
    //     area.set_grows_down(true);
    // }

    // ——————————————————————————————————————————
    // 11. 插入到 MemorySet 的 areatree / VMA 列表
    ms.areatree.push(area);
    flush_tlb();

    // 12. 返回映射基址
    Ok(base.0)
}

/// YOUR JOB: Implement munmap.
pub async  fn sys_munmap(start: usize, len: usize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_munmap ", current_task().get_pid());
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    current_process()
        .memory_set
        .lock().await
        .munmap(start_va.floor(), end_va.ceil());
    Ok(0)
}

/// change data segment size
pub async  fn sys_sbrk(size: i32) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_sbrk size:{:#x}",
        current_task().get_pid(),
        size
    );
match current_process().change_program_brk(size).await {
        Some(old_brk) => Ok(old_brk),
        None => Err(SysErrNo::ENOMEM),
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> SyscallRet {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().get_pid()
    );
    unimplemented!();
    // let current_task = current_task();
    // let token =current_task.get_user_token();
    // let path = translated_str(token, _path);
    // let elf;
    // if let Ok(app_inode) = open_file(path.as_str(), OpenFlags::O_RDONLY,0o777) {
    //    elf = app_inode.file().unwrap().read_all();
    //    let tcb=Arc::new(TaskControlBlock::new(elf.as_slice(),path));
    //    let mut inner=tcb.inner_exclusive_access();
    //    let mut pin=current_task.inner_exclusive_access();
    //    inner.parent.replace(current_task.get_pid());
    //    pin.children.push(tcb.clone());
    //    drop(inner);
    //    let pid = tcb.get_pid() as isize;
    //    add_task(tcb);
    //    return pid;
    // }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> SyscallRet {
    trace!("kernel:pid[{}] sys_set_priority ", current_task().get_pid());

    set_priority(&(*current_task().0), prio);
    Ok(0)
}
pub fn sys_geteuid() -> SyscallRet {
   
        trace!(
            "kernel:pid[{}] sys_getuid NOT IMPLEMENTED",
            current_task().get_pid()
        );
        //todo(heliosly)
        Ok(0)
    
}

/// getcwd 系统调用实现
/// buf: 用户空间缓冲区的指针，用于存储当前工作目录路径
/// size: 用户缓冲区的大小
/// 返回：成功时为实际写入用户缓冲区的字节数（包括末尾的 '\0'），
///       如果 size 太小，返回 -ERANGE，
///       如果 buf 无效，返回 -EFAULT，
///       其他错误则返回相应错误码。
/// POSIX getcwd 成功时返回 buf 指针，失败时返回 NULL。
/// 我们这里调整为返回写入的长度或错误码。
pub async fn sys_getcwd(buf_user_ptr: *mut u8, size: usize) -> SyscallRet {
    // trace!("sys_getcwd(buf: {:p}, size: {})", buf_user_ptr, size);

    if buf_user_ptr.is_null() && size != 0 { // POSIX 允许 buf 为 NULL 以查询所需大小，但我们这里简化
        let len =current_process() .cwd.lock().await.as_bytes().len() + 1; // 包括 null terminator
        return Ok(len);
    }
    if size == 0 && !buf_user_ptr.is_null() { // size 为0但buf非NULL，POSIX行为未明确，这里视为无效
        return Err(SysErrNo::EINVAL);
    }
    if size == 0 && buf_user_ptr.is_null() { // POSIX 允许此情况动态分配，我们不支持
        return Err(SysErrNo::EINVAL); // 或者 ERANGE，因为大小为0肯定不够
    }
    let proc_arc = current_process();
    let cwd_kernel_string = proc_arc.cwd.lock().await.clone();
    let token = proc_arc.get_user_token().await; // 或者 proc_arc.memory_set.lock().await.token();
    let cwd_len_with_null = cwd_kernel_string.len() + 1;

    if size < cwd_len_with_null {
        return Err(SysErrNo::ERANGE);
    }

    // 1. 创建 buffer_to_copy，使其生命周期覆盖 copy_to_user_bytes 调用
    let mut buffer_to_copy = Vec::with_capacity(cwd_len_with_null);
    buffer_to_copy.extend_from_slice(cwd_kernel_string.as_bytes());
    buffer_to_copy.push(0); // Null terminator

    // 2.  bytes_to_copy_slice 借用一个仍然存活的 buffer_to_copy
    let bytes_to_copy_slice: &[u8] = &buffer_to_copy;

    // 3. 使用 copy_to_user_bytes
    match unsafe {
        copy_to_user_bytes(
            token,
            VirtAddr::from(buf_user_ptr as usize),
            bytes_to_copy_slice,
        )
    } {
        Ok(bytes_copied) => {
            if bytes_copied != cwd_len_with_null {
                // log::error!("sys_getcwd: copy_to_user_bytes copied {} bytes, expected {}", bytes_copied, cwd_len_with_null);
                Err(SysErrNo::EFAULT)
            } else {
                Ok(bytes_copied )
            }
        }
        Err(translate_error) => {
            // log::warn!("sys_getcwd: copy_to_user_bytes failed: {:?}", translate_error);
            match translate_error {
                TranslateRefError::TranslationFailed(_) | TranslateRefError::PermissionDenied(_) => Err(SysErrNo::EFAULT),
                TranslateRefError::UnexpectedEofOrFault => Err(SysErrNo::EIO),
                _ => Err(SysErrNo::EFAULT),
            }
        }
    }
}