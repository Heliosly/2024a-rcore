//! Process management syscalls
//!

use alloc::sync::Arc;

use crate::{
    config::{MAX_SYSCALL_NUM, PAGE_SIZE},  fs::{open_file, OpenFlags}, mm::{translated_byte_buffer, translated_refmut, translated_str, MapPermission, VirtAddr}, task::{
        add_task, current_task, current_user_token, exit_current_and_run_next, set_priority, yield_now, TaskControlBlock, TaskStatus
    }, timer::get_time_us
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
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);

    exit_current_and_run_next(exit_code);
    exit_code as isize
}

pub async  fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    yield_now().await;
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();

    let new_task = current_task.fork();

    let new_pid = new_task.pid.0;
    debug!("new_pid:{}",new_pid);
  
    // modify trap context of new_task, because it returns immediately after switching
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    // add new task to scheduler
    let inner = new_task.inner_exclusive_access();
    for area in inner.memory_set.areas.iter(){
        for (_,ppn) in area.data_frames.iter(){
            if ppn.ppn().0==0x81901 {
                print!("\nfork in pid{}\n",new_pid);
            }
            if new_task.pid.0==3 {
                println!("in fork pid =3 ,ppn={:#x}",ppn.ppn().0);
            }
        }
    }
    drop(inner);
    add_task(new_task);
   
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);

    let token = current_user_token();
    let path = translated_str(token, path);
    if let Ok(app_inode) = open_file(path.as_str(), OpenFlags::O_RDONLY,0o777) {
        let all_data = app_inode.file().unwrap().read_all();
        let task = current_task().unwrap();
        let inner = task.inner_exclusive_access();
        
        drop(inner); 
        task.exec(all_data.as_slice());
        let inner= task.inner_exclusive_access();
for area in inner.memory_set.areas.iter(){
            for (_,ppn) in area.data_frames.iter(){
                if ppn.ppn().0==0x81901 {
                    print!("\nexec in pid{}\n",task.pid.0);
                }
                if task.pid.0==3 {
                    println!("in exec pid =3 ,ppn={:#x}",ppn.ppn().0);
                }
            }
        }
         inner.memory_set.activate();       
         drop(inner);
         0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    
    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();

    // for i in inner.children.iter(){
    //     print!(" {}",{i.pid.0}); 
    // }

        // println!(" ");
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        debug!("cant find pid:{} in parent pid:{},and children count = : {} ",
        pid,task.pid.0,inner.children.len());
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        debug!("chiled idx is removed");
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;

        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        assert_eq!(Arc::strong_count(&child), 1);
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
        current_task().unwrap().pid.0
    );
    let usec=get_time_us().to_ne_bytes();
   
   
    let sec=(get_time_us()/1000000).to_ne_bytes();

    let _cur=current_user_token();
    
    let bufs=translated_byte_buffer(_cur, _ts as *const u8, 16);
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
        current_task().unwrap().pid.0
    );
    -1
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    if _start % PAGE_SIZE != 0 || _port & !0x7 != 0 || _port & 0x7 == 0 {
        return -1;  
    }
    let arc=current_task().unwrap();
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
        current_task().unwrap().pid.0
    );
    if _start % PAGE_SIZE != 0  {
        return -1;  
    }
    let _end=_start+_len;
    let end:VirtAddr=_end.into();
    let start:VirtAddr=_start.into();
    let arc=current_task().unwrap();
    
    if arc.inner_exclusive_access().memory_set.unmap_peek(start, end){
        return -1
    }
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
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
        current_task().unwrap().pid.0
    );
    let current_task = current_task().unwrap();
    let token =current_user_token();
    let path = translated_str(token, _path);
    let elf;
    if let Ok(app_inode) = open_file(path.as_str(), OpenFlags::O_RDONLY,0o777) {
       elf = app_inode.file().unwrap().read_all();
       let tcb=Arc::new(TaskControlBlock::new(elf.as_slice()));
       let mut inner=tcb.inner_exclusive_access();
       let mut pin=current_task.inner_exclusive_access();
       inner.parent=Some(Arc::downgrade(&current_task));
       pin.children.push(tcb.clone());
       drop(inner);
       let pid = tcb.pid.0 as isize;
       add_task(tcb);
       return pid;
    } 
    -1
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
   
   set_priority(current_task().unwrap(), prio);
   0
}
