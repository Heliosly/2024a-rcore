//! Process management syscalls
//!

use core::panic;

use alloc::{
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::{
    config::{MAX_SYSCALL_NUM, MMAP_TOP, PAGE_SIZE}, fs::{open_file, OpenFlags, NONE_MODE}, mm::{
        flush_tlb, translated_byte_buffer, translated_ref, translated_refmut, translated_str, MapArea, MapAreaType, MapPermission, MapType, VirtAddr, VirtPageNum
    }, syscall::flags::{MmapFlags, MmapProt}, task::{
         current_process, current_task, current_token, exit_current_and_run_next,
        set_priority, yield_now, CloneFlags,  TaskStatus, PID2PC,
    }, timer::{get_time_ms, get_time_us}, utils::{error::SysErrNo, page_round_up, string::get_abs_path}
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

pub fn sys_getppid()->isize{
    trace!("[sys_getppid] pid :{}", current_task().get_pid());
    current_process().parent() as isize
}
pub fn sys_exit(exit_code: i32) -> isize {
    trace!("kernel:pid[{}] sys_exit", current_task().get_pid());

    exit_current_and_run_next(exit_code);
    exit_code as isize
}

pub async fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    yield_now().await;
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().get_pid());
    current_task().get_pid() as isize
}
/// # Arguments for riscv
/// * `flags` - usize
/// * `user_stack` - usize
/// * `ptid` - usize
/// * `tls` - usize
/// * `ctid` - usize
pub async  fn sys_clone(args: [usize; 6]) -> isize {
    //解析参数
    let flags = args[0];
    let user_stack = args[1];
    let ptid = args[2];
    let tls = args[3];
    let ctid = args[4];
    let proc = current_process();

    let flags =
        CloneFlags::from_bits(flags & !0x3f).expect(&format!("unsupport cloneflags : {}", flags));
    debug!("[sys_clone] flags:{:#?},user_stack:{:#x},ptid:{:#x},tls:{:#x},ctid:{:#x}",flags,user_stack,ptid,tls,ctid);
    if flags.contains(CloneFlags::CLONE_SIGHAND) && !flags.contains(CloneFlags::CLONE_VM) {
        // Error when CLONE_SIGHAND was specified in the flags mask, but CLONE_VM was not.
        return SysErrNo::EINVAL as isize;
    }
    if flags.contains(CloneFlags::CLONE_PIDFD) && !flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
        return SysErrNo::EINVAL as isize;
    }

   proc.clone_task(flags,user_stack,ptid,tls,ctid).await

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
pub fn sys_execve(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().get_pid());
    let process = current_process();

    let token = process.get_user_token();
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
        let argv_ptr = *translated_ref(token, argv);
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
        let envp_ptr = *translated_ref(token, envp);
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
                let cwd_lock = process.cwd.lock();
                cwd_lock.clone()
            } else {
                "/".to_string()
            };
    let abs_path = get_abs_path(&cwd, &path);
    let app_inode = match open_file(&abs_path, OpenFlags::O_RDONLY, NONE_MODE) {
        Ok(file) =>  file.file().unwrap(),
        
        Err(err) => {
            println!("the file is not existed");
            return err as isize;
        }
    };
    
    let elf_data = app_inode.read_all();
    process.exec(&elf_data, &argv_vec, &mut env);
    process.memory_set.lock().activate();
    argv_vec.len() as isize
}

pub fn sys_settidaddress()->isize{
    trace!(
        "kernel:pid[{}] sys_settidaddress NOT IMPLEMENTED",
        current_task().get_pid()
    );
    //todo(Heliosly)
    current_task().id.as_usize() as isize
}
pub fn sys_getuid()->isize{
  trace!(
        "kernel:pid[{}] sys_getuid NOT IMPLEMENTED",
        current_task().get_pid()
    );
    //todo(heliosly)
  0
}
/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel: sys_waitpid");
    let proc = current_process();
    // find a child process

    // ---- access current PCB exclusively

    // for i in inner.children.iter(){
    //     print!(" {}",{i.get_pid()});
    // }

    // println!(" ");

    let mut childvec = proc.children.lock();
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
        return -1;
        // ---- release current PCB
    }
    let pair = childvec.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.is_zombie() && (pid == -1 || pid as usize == p.get_pid())
        // ++++ release child PCB
    });
    if let Some((idx, child_task)) = pair {
        debug!("chiled idx is removed");

        PID2PC.lock().remove(&child_task.get_pid());
        let child = childvec.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        let found_pid = child.get_pid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.exit_code();

        // ++++ release child PCB
        *translated_refmut(proc.get_user_token(), exit_code_ptr)=exit_code ;
        assert_eq!(
            Arc::strong_count(&child),
            1,
            "strong_count is incorrect,{}",
            Arc::strong_count(&child)
        );
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
        current_task().get_pid()
    );
    let usec = get_time_us().to_ne_bytes();

    let sec = (get_time_us() / 1000000).to_ne_bytes();

    let token = current_token();

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
    0
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

pub fn sys_exitgroup(exit_code: i32) -> isize {
    trace!(
        "kernel:pid[{}] sys_exit_exitgroup NOT IMPLEMENTED",
        current_task().get_pid()
    );
    exit_current_and_run_next(exit_code);
       0
}
/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!(
        "kernel:pid[{}] sys_task_info ",
        current_task().get_pid()
    );
    -1
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
) -> isize {
    
    //需要有严格的判断返回错误的顺序！！！
     // 0. flags 不能全 0
     if flags == 0 {
        return SysErrNo::EINVAL as isize;
    }
    let flags = MmapFlags::from_bits(flags).unwrap();

    // // 1. 长度不能为 0
    // if len == 0 {
    //     return SysErrNo::EINVAL as isize;
    // }
    // // 2. 偏移量必须页对齐
    // if off % PAGE_SIZE != 0 {
    //     return SysErrNo::EINVAL as isize;
    // }

    // 3. 匿名映射 | 文件映射：fd 检查
    let anon = flags.contains(MmapFlags::MAP_ANONYMOUS);
    if !anon {
        // 非匿名必须提供合法 fd
        if fd == usize::MAX {
            return SysErrNo::EBADF as isize;
        }
    }
    // 4. prot -> MapPermission
    let mmap_prot = MmapProt::from_bits(prot).unwrap();
    let map_perm: MapPermission = mmap_prot.into();
    debug!(
        "[sys_mmap]: addr {:#x}, len {:#x}, fd {}, offset {:#x}, flags {:?}, prot is {:?}, map_perm {:?}",
        addr, len, fd as isize, off, flags,mmap_prot, map_perm
    );
    // 5. MAP_FIXED 且 addr == 0 禁止
    if flags.contains(MmapFlags::MAP_FIXED) && addr == 0 {
        return SysErrNo::EPERM as isize;
    }

    // 6. 计算页对齐后的长度和页数
    let len = page_round_up(len);
    let pages = len / PAGE_SIZE;

    let proc = current_process();
    let mut ms = proc.memory_set.lock();

    let fd_table =proc.fd_table.lock();
    // ——————————————————————————————————————————
    // 7. 如果是文件映射，要检查文件权限和 offset
    let file = if !anon {
        // 7.1 拿到文件对象
        let file = match fd_table.get(fd).unwrap() {
            Some(f) => f,
            None    => return SysErrNo::EBADF as isize,
        };
        // 7.2 写映射需可写权限
        if map_perm.contains(MapPermission::W) && !file.writable().await.unwrap() {
            return SysErrNo::EACCES as isize;
        }
        // 7.3 offset 超出文件长度？
        if off > file.fstat().st_size as usize {
            return SysErrNo::EINVAL as isize;
        }
        
        unimplemented!();
        Some(file.clone())
    } else {
        None
    };

    let va = VirtAddr::from(addr);
    let vpn = va.floor();
    let end_vpn = VirtPageNum::from( vpn.0+pages);
    let range=core::ops::Range { start: (vpn), end: (end_vpn)};
    // ——————————————————————————————————————————
    // 8. 按 MAP_FIXED / MAP_FIXED_NOREPLACE / hint / 自动选择地址
    let base = if flags.contains(MmapFlags::MAP_FIXED) {
        // 8.1 强制定位：先 munmap 冲突区

        
        if ms.areatree.is_overlap(&range )
            {

                ms.munmap(vpn, end_vpn);
            }
        
        va
    } else if flags.contains(MmapFlags::MAP_FIXED_NOREPLACE) {
        // 8.2 不替换：如果冲突则失败
        if ms.areatree.is_overlap(&range) {

            return SysErrNo::EEXIST as isize;
        }
        va
    } else {
        // 8.3 默认，从 addr 附近或全局搜索
       match  ms.areatree.alloc_pages(pages){
        Some(vpn) => {
            VirtAddr::from(vpn)
        },
        None => {
            warn!("[sys_mmap] no available gap for mapping {} pages from {:?}", pages, vpn);
            return SysErrNo::ENOMEM as isize;
        } 
       } 
       
    };
    if base.0>=MMAP_TOP{
        return SysErrNo::ENOMEM as isize;
    }
    // ——————————————————————————————————————————
    // 9. 构造 VMA / MapArea
    let mut area = MapArea::new(
        base,
        VirtAddr::from(base.0 + len),
        MapType::Framed,
        map_perm,
        MapAreaType::Mmap,   // Option<Arc<File>>
        
    );

    debug!("[sys_mmap]mmap ok,base:{:#x}",base.0);

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
    base.0 as isize 
}


/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap ",
        current_task().get_pid()
    );
    let start_va=VirtAddr::from(start);
    let end_va= VirtAddr::from(start+len);
    current_process().memory_set.lock().munmap(start_va.floor(),end_va.ceil() );
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk size:{:#x}", current_task().get_pid(),size);
    if let Some(old_brk) = current_process().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
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
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority ",
        current_task().get_pid()
    );

    set_priority(&(*current_task().0), prio);
    0
}
