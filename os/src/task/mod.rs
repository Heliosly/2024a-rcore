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

pub mod aux;
mod flags;
mod current;
mod id;
mod kstack;
mod processor;
mod schedule;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
pub(crate) mod waker;
pub mod sleeplist;
mod yieldfut;
use alloc::boxed::Box;
// mod timelist;
use alloc::vec;
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
pub use yieldfut::yield_now;
pub use waker::custom_noop_waker;

// pub use manager::get_task_count;
use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;



use spin::mutex::Mutex as Spin;
// pub use manager::{fetch_task, TaskManager,pick_next_task};

// pub use manager::add_task;

pub type ProcessRef = Arc<ProcessControlBlock>;
pub static PID2PC: Spin<BTreeMap<usize, ProcessRef>> =Spin::new(BTreeMap::new());
pub static TID2TC: Spin<BTreeMap<usize, TaskRef>> = Spin::new(BTreeMap::new());
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    panic!("undo");
    // There must be an application running.
    // let task = take_current_task().unwrap();

    // // ---- access current TCB exclusively
    // let mut task_inner = task.inner_exclusive_access();
    // let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // // Change status to Ready
    // task_inner.set_state(TaskStatus::Runable);
    // drop(task_inner);
    // // ---- release current PCB

    // // push back to ready queue.
    // add_task(task);
    // // jump to scheduling cycle
    // schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Exit the current 'Running' task and run the next task in task list.
pub async  fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = current_task();

    let process = current_process();

    println!("[kernel]exit pid {},exit code:{}", task.get_pid(), exit_code);
    // **** access current TCB exclusively
    // Change status to Zombie
    task.set_state(TaskStatus::Zombie);
    // Record exit code
    task.set_exit_code(exit_code as isize);
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
    process.fd_table.lock().await.clear();

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

    

    // drop task manually to maintain rc correctly
    drop(task); 
}

// static INITPROC_STR: &str =          "ch5b_user_shell";
// static INITPROC_STR: &str =          "ch2b_power_3";
// static INITPROC_STR: &str =          "musl/basic/yield";
 static INITPROC_STR: &str =          "musl/busybox";

//  static INITPROC_STR: &str =          "cosmmap_clone";
//  static INITPROC_STR: &str =          "cosshell";
pub static INITPROC :LazyInit<ProcessRef> = LazyInit::new();
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.

//  pub  static KERNEL_IDLE_PROCESS:LazyInit<ProcessRef> = LazyInit::new();


    pub fn add_initproc() {
        let inode = open_file(INITPROC_STR, OpenFlags::O_RDONLY, 0o777).unwrap();
        let data = inode.file().unwrap().read_all();
    
        let mut envs = get_envs(); // 注意：new 需要 &mut envs
        let binding = get_args("musl/busybox sh".as_bytes());
        let pcb_fut = ProcessControlBlock::new(
            data.as_slice(),
            "/".to_string(),
            &binding,
            &mut envs,
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