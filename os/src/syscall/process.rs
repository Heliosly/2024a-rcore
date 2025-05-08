//! Process management syscalls
//!

use alloc::{string::{String, ToString}, sync::Arc, vec::Vec};

use crate::{
    config::{MAX_SYSCALL_NUM, PAGE_SIZE},  fs::{open_file, OpenFlags, NONE_MODE}, mm::{translated_byte_buffer, translated_ref, translated_refmut, translated_str, FrameTracker, MapPermission, VirtAddr}, task::{
        add_task, current_process, current_task, current_token, exit_current_and_run_next, set_priority, yield_now, ProcessControlBlock, TaskStatus, PID2PC
    }, timer::get_time_us, utils::string::get_abs_path
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

pub fn sys_exit(exit_code: i32)->isize  {
    trace!("kernel:pid[{}] sys_exit", current_task().get_pid());

    exit_current_and_run_next(exit_code);
    exit_code as isize
}

pub async  fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    yield_now().await;
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().get_pid());
    current_task().get_pid() as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().get_pid());
    let current_process = current_process();

    let new_pro = current_process.fork();
    let new_pid = new_pro.get_pid();
    // debug!("new_pid:{}",new_pid);
  
    // modify trap context of new_task, because it returns immediately after switching
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    // add new task to scheduler
    // let inner = new_task.inner_exclusive_access();
    // for area in inner.memory_set.areas.iter(){
    //     for (_,ppn) in area.data_frames.iter(){
    //         if ppn.ppn().0==0x81901 {
    //             print!("\nfork in pid{}\n",new_pid);
    //             println!("in fork pid =3 ,ppn={:#x},frametracker_ptr:{:#x}",
    //             ppn.ppn().0,ppn as *const _ as usize);
    //         }
    //         if new_task.get_pid()==3 {
    //             println!("in fork pid =3 ,ppn={:#x},frametracker_ptr:{:#x}",
    //             ppn.ppn().0,ppn as *const _ as usize);
    //         }
    //     }
    // }
    // drop(inner);
   
    new_pid as isize
}

/// 参考 https://man7.org/linux/man-pages/man2/execve.2.html
pub fn sys_execve(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> isize{
    trace!("kernel:pid[{}] sys_exec", current_task().get_pid());
    let process = current_process();

    let token = process.get_user_token();
    let mut path = translated_str(token, path);
    let proc_inner = process.inner_exclusive_access();
    // path = remove_ansi_escape_sequences(&path);
    // path = strip_color(path, "\u{1b}[0;0m", "\u{1b}[m");
    // if path.starts_with("ltp/testcases/bin/\u{1b}[1;32m") {
    //     //去除颜色
    //     path = strip_color(path, "ltp/testcases/bin/\u{1b}[1;32m", "\u{1b}[m");
    // }
    //log::info!("[sys_execve] path={}", path);

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
        proc_inner.cwd.as_str()
    } else {
        "/"
    };
    let abs_path = get_abs_path(&cwd, &path);
    let app_inode = open_file(&abs_path, OpenFlags::O_RDONLY, NONE_MODE).unwrap().file().unwrap();
    let elf_data = app_inode.read_all();
    drop(proc_inner);
    process.exec(&elf_data, &argv_vec, &mut env);
    process.inner_exclusive_access().memory_set.activate();
    0


}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel: sys_waitpid");
    let proc= current_process();
    // find a child process

    
    // ---- access current PCB exclusively
    let mut proc_inner = proc.inner_exclusive_access();

    // for i in inner.children.iter(){
    //     print!(" {}",{i.get_pid()}); 
    // }

        // println!(" ");
    if !proc_inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.get_pid())
    {

        debug!("cant find pid:{} in parent pid:{},and children count = : {} ",
        pid,proc.get_pid(),proc_inner.children.len());
        return -1;  
        // ---- release current PCB
    }
    let pair = proc_inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.is_zombie() && (pid == -1 || pid as usize == p.get_pid())
        // ++++ release child PCB
    });
    if let Some((idx, child_task)) = pair {
        debug!("chiled idx is removed");

        PID2PC.lock().remove(&child_task.get_pid());
        let child = proc_inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        let found_pid = child.get_pid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;

        // ++++ release child PCB
        *translated_refmut(proc_inner.memory_set.token(), exit_code_ptr) = exit_code;
        assert_eq!(Arc::strong_count(&child), 1,"strong_count is incorrect,{}",
        Arc::strong_count(&child));
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
    let usec=get_time_us().to_ne_bytes();
   
   
    let sec=(get_time_us()/1000000).to_ne_bytes();

    let token=current_token();
    
    let bufs=translated_byte_buffer(token, _ts as *const u8, 16);
    let mut i=0;
    for buf in bufs{
        for atm in buf{
            if i>=8{
                *atm = usec[i-8];
                }
                else{
                *atm=sec[i];
                }
        
                i+=1;
        }
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!(
        "kernel:pid[{}] sys_task_info NOT IMPLEMENTED",
        current_task().get_pid()
    );
    -1
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap NOT IMPLEMENTED",
        current_task().get_pid()
    );
    if _start % PAGE_SIZE != 0 || _port & !0x7 != 0 || _port & 0x7 == 0 {
        return -1;  
    }
    let arc=current_process();
    bitflags! {
        /// map permission corresponding to that in pte: `R W X U`
        pub struct Portpomiss: u8 {
            ///Readable
            const R = 1 << 0;
            ///Writable
            const W = 1 << 1;
            ///Excutable
            const X = 1 << 2;
            
        }
    }
    let portpomis = Portpomiss::from_bits_truncate(_port as u8);
    let mut flag:MapPermission=MapPermission::empty();
    flag|=MapPermission::U;
    if portpomis.contains(Portpomiss::R){
         flag|=MapPermission::R;
    }
    if portpomis.contains(Portpomiss::W){
         flag|=MapPermission::W;
    }
    if portpomis.contains(Portpomiss::X){
        flag|=MapPermission::X;
    }
    let _end=_start+_len;
   
    let end:VirtAddr=_end.into();
    let start:VirtAddr=_start.into();
    if  arc.inner_exclusive_access().memory_set.insert_framed_area_peek_for_mmap(start, end, flag)
    {-1}
    else{
        0
    }
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap NOT IMPLEMENTED",
        current_task().get_pid()
    );
    if _start % PAGE_SIZE != 0  {
        return -1;  
    }
    let _end=_start+_len;
    let end:VirtAddr=_end.into();
    let start:VirtAddr=_start.into();
    let arc=current_process();
    
    if arc.inner_exclusive_access().memory_set.unmap_peek(start, end){
        return -1
    }
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().get_pid());
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
    -1
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().get_pid()
    );
   
   set_priority(&(*current_task().0), prio);
   0
}
