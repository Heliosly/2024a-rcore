//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
#![allow(missing_docs)]

// mod tls;
pub mod aux;
mod flags;
mod current;
mod id;
mod kstack;
mod processor;
pub mod future;
pub mod fdmanage;
mod schedule;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
pub(crate) mod waker;
pub mod sleeplist;

use alloc::boxed::Box;
// mod timelist;
use alloc::{format, vec};
use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use alloc::string::{String, ToString};
use lazy_init::LazyInit;
use core::future::Future;
pub use core::mem::ManuallyDrop;
use core::task::{Context, Poll};
pub use current::{
    current_process, current_task, current_task_may_uninit, current_token,current_task_id,
    CurrentTask,current_task_id_may_uninit,
};
pub use flags::{CloneFlags,TaskStatus};

pub use id::{pid_alloc, PidHandle, RecycleAllocator};
pub use kstack::{TaskStack,current_stack_top};
pub use processor::{init, run_task2};
pub use schedule::{add_task, pick_next_task, put_prev_task, set_priority, task_tick,Task,TaskRef};
pub use task::ProcessControlBlock;
pub use future::yield_now;
pub use waker::custom_noop_waker;

pub use task::RobustList;
// pub use manager::get_task_count;
use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;



use spin::mutex::Mutex as Spin;
// pub use manager::{fetch_task, TaskManager,pick_next_task};
#[inline]
pub unsafe fn write_thread_pointer(tp: usize) {
    core::arch::asm!("mv tp, {}", in(reg) tp)
}

// pub use manager::add_task;
// pub fn init_tls() {
//     let main_tls = tls::TlsArea::alloc();
//     unsafe {write_thread_pointer(main_tls.tls_ptr() as usize) };
//     core::mem::forget(main_tls);
// }
pub type ProcessRef = Arc<ProcessControlBlock>;
pub static PID2PC: Spin<BTreeMap<usize, ProcessRef>> =Spin::new(BTreeMap::new());
pub static TID2TC: Spin<BTreeMap<usize, TaskRef>> = Spin::new(BTreeMap::new());
pub fn task_count()->usize{
    TID2TC.lock().len()
}
/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;
/// 整个进程退出：把进程里所有线程一起结束，回收所有资源
pub async fn exit_proc(exit_code: i32) {
    // 1. 拿到当前进程的 PCB
    let process = current_process();
    let pid = process.get_pid();

    debug!("[kernel] exit_proc pid {}, exit_code: {}", pid, exit_code);

    // 2. 先把所有线程都标记为 Zombie、设置退出码、清除 child_tid
    {
        let  tasks = process.tasks.lock().await;
        for thread in tasks.iter() {
            thread.set_state(TaskStatus::Zombie);
            thread.set_exit_code(exit_code as isize);
            let _ = thread.clear_child_tid(); 
            // TODO(Heliosly)“唤醒”线程
        }
    }

    // 3. 如果有子进程，要把它们 reparent 给 init
    if pid != 1 {
        let mut children = process.children.lock().await;
        for child in children.iter() {
            child.set_parent(INITPROC.pid.0);
            INITPROC.children.lock().await.push(child.clone());
        }
        children.clear();
    }

    // 4. 回收“进程级”资源：地址空间、FD 表、信号等
    {
        // 回收地址空间的所有用户页
        process.memory_set.lock().await.recycle_data_pages();
        // 关闭并清空文件
        process.fd_table.lock().await.table.clear();
        // （如果有其他全局结构，比如信号队列、管道、TLS 等，也要在这里清理）
    }

    // 5. 从全局 TID2TC、tasks 列表里把所有线程移除
    {
        let mut tasks = process.tasks.lock().await;
        for thread in tasks.iter() {
            let tid = thread.id.as_usize();
            TID2TC.lock().remove(&tid);
            // drop(thread) 由 Rust 智能指针自动处理
        }
        tasks.clear();
    }


    //   
    // let ppid = process.get_parent();
    // if let Some(parent_proc) = PID2PC.lock().get(&ppid) {
    //     parent_proc.notify_child_exited(pid, exit_code);
    // }

    
}
///Exit the current 'Running' task and run the next task in task list. 
/// 并不清理资源等waitpid回收
pub async  fn exit_current(exit_code: i32) {
    // take from Processor
    let task = current_task();
    
    let process = current_process();

    debug!("[kernel]exit pid {},exit code:{}", task.get_pid(), exit_code);
    // **** access current TCB exclusively
    // Change status to Zombie
    task.set_state(TaskStatus::Zombie);
    // Record exit code
    task.set_exit_code(exit_code as isize);
    task.clear_child_tid().unwrap();
    process.set_exit_code(exit_code as i32);
    let tid = task.id.as_usize();
    if task.is_leader() {
        if task.get_pid() != 1 {
            for child in process.children.lock().await.iter() {
                child
                    .set_parent(INITPROC.pid.0) ;
                INITPROC.children.lock().await.push(child.clone());
            }

        } else {
            
            let w = &INITPROC;
            let _ = w;
        }


    // ++++++ release parent PCB

    process.children.lock().await.clear();
    // deallocate user space
    process.memory_set.lock().await.recycle_data_pages();
    // drop file descriptors
    process.fd_table.lock().await.table.clear();

    } 
    TID2TC.lock().remove(&tid);
    
    // 从进程中删除当前线程
    let mut tasks = process.tasks.lock().await;
    let len = tasks.len();
    for index in 0..len {
        if tasks[index].id.as_usize() == tid {
            tasks.remove(index);
            break;
        }
    }
    drop(task); 
}

// static INITPROC_STR: &str =          "ch5b_user_shell";
// static INITPROC_STR: &str =          "ch2b_power_3";
// static INITPROC_STR: &str =          "/glibc/basic/brk";
 static INITPROC_STR: &str =          "/glibc/busybox";

//  static INITPROC_STR: &str =          "/tls";

//  static INITPROC_STR: &str =          "/glibc/busybox";
//  static INITPROC_STR: &str =          "cosmmap_clone";
//  static INITPROC_STR: &str =          "cosshell";
pub static INITPROC :LazyInit<ProcessRef> = LazyInit::new();
static  CWD:&str = "/glibc";
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.

//  pub  static KERNEL_IDLE_PROCESS:LazyInit<ProcessRef> = LazyInit::new();


pub fn add_initproc(cwd:&str,exe:&str,argv:&str){
        let inode = open_file(exe, OpenFlags::O_RDONLY, 0o777).unwrap();
        let data = inode.file().unwrap().read_all();
    
        let mut envs = get_envs(); // 注意：new 需要 &mut envs
        let binding = get_args(
            format!("{} {} ",exe,argv)
            
        .as_bytes());
        let pcb_fut = ProcessControlBlock::new(
            data.as_slice(),
            cwd.to_string(),
            &binding,
            &mut envs,
            INITPROC_STR.to_string(),
        );
    
        // . Pin 到堆上
        let mut pinned = Box::pin(pcb_fut);
    
        let waker = custom_noop_waker();
        let mut ctx = Context::from_waker(&waker);
    
        let pcb_inner = match pinned.as_mut().poll(&mut ctx) {
            Poll::Ready(pcb) => pcb,
            Poll::Pending => {
                panic!("KERNEL_ASSERTION_FAILURE: ProcessControlBlock::new returned Pending");
            }
        };
    
        //  用结果初始化
        let pcb = Arc::new(pcb_inner);
        INITPROC.init_by(pcb.clone());
        PID2PC.lock().insert(INITPROC.pid.0, pcb);
    
        trace!("add_initproc ok");
    }

    

  
#[allow(unused)]
/// 分割命令行参数，支持双引号
fn get_args(command_line: &[u8]) -> Vec<String> {
    let mut args = Vec::new();
    let mut in_quote = false;
    let mut buf: Vec<u8> = Vec::new();

    for &b in command_line {
        match b {
            b'"' => {
                // 遇到引号就切换 in_quote 状态，不加入 buf
                in_quote = !in_quote;
            }
            b' ' if !in_quote => {
                // 外层空格，分隔参数
                if !buf.is_empty() {
                    // 将当前缓冲区作为一个完整参数
                    args.push(String::from_utf8(buf.clone()).unwrap());
                    buf.clear();
                }
                // 否则跳过多余空格
            }
            _ => {
                // 普通字符或引号内的空格，加入 buf
                buf.push(b);
            }
        }
    }

    // 最后一个参数
    if !buf.is_empty() {
        args.push(String::from_utf8(buf).unwrap());
    }

    args
}
#[allow(dead_code)]
const BUSYBOX_TESTCASES: &[&str] = &[
    "busybox sh busybox_testcode.sh",
    "busybox sh lua_testcode.sh",
    "libctest_testcode.sh",
];

#[allow(dead_code)]
const TESTCASES: &[&str] = &[
    "batch_syscall",
    // "syscall_test",
    // "vdso_test",
    // "hello_world",
    // "pipetest",
    // "std_thread_test",
];
/// Now the environment variables are hard coded, we need to read the file "/etc/environment" to get the environment variables
pub fn get_envs() -> Vec<String> {
    // Const string for environment variables
    let  envs:Vec<String> = vec![
        "SHLVL=1".into(),
        "PWD=/".into(),
        "GCC_EXEC_PREFIX=/riscv64-linux-musl-native/bin/../lib/gcc/".into(),
        "COLLECT_GCC=./riscv64-linux-musl-native/bin/riscv64-linux-musl-gcc".into(),
        "COLLECT_LTO_WRAPPER=/riscv64-linux-musl-native/bin/../libexec/gcc/riscv64-linux-musl/11.2.1/lto-wrapper".into(),
        "COLLECT_GCC_OPTIONS='-march=rv64gc' '-mabi=lp64d' '-march=rv64imafdc' '-dumpdir' 'a.'".into(),
        "LIBRARY_PATH=/lib/".into(),
        "LD_LIBRARY_PATH=/lib/".into(),
        "LD_DEBUG=files".into(),
    ];

    envs
}