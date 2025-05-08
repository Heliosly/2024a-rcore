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
mod current;
mod id;
mod kstack;
mod processor;
mod schedule;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
mod waker;
mod yieldfut;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::ToString;
pub use core::mem::ManuallyDrop;
pub use current::{
    current_process, current_task, current_task_may_uninit, current_token,
    CurrentTask,
};
pub use id::{pid_alloc, PidHandle, RecycleAllocator};
pub use kstack::{TaskStack,current_stack_top};
pub use processor::{init, run_task2, run_tasks, Processor};
pub use schedule::{add_task, pick_next_task, put_prev_task, set_priority, task_tick};
pub use task::{ProcessControlBlock, TaskStatus};
pub use yieldfut::yield_now;
// pub use manager::get_task_count;
use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;


use lazy_static::*;

use schedule::TaskRef;
use spin::mutex::Mutex;
// pub use manager::{fetch_task, TaskManager,pick_next_task};

// pub use manager::add_task;

pub type ProcessRef = Arc<ProcessControlBlock>;
pub static PID2PC: Mutex<BTreeMap<usize, ProcessRef>> = Mutex::new(BTreeMap::new());
pub static TID2TC: Mutex<BTreeMap<usize, TaskRef>> = Mutex::new(BTreeMap::new());
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
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = current_task();

    let process = current_process();

    println!("[kernel]exit pid {},exit code:{}", task.get_pid(), exit_code);
    // **** access current TCB exclusively
    let mut inner = process.inner_exclusive_access();
    // Change status to Zombie
    task.set_state(TaskStatus::Zombie);
    // Record exit code
    task.set_exit_code(exit_code as isize);
    let tid = task.id.as_usize();
    if task.is_leader() {
        if task.get_pid() != 0 {
            let mut initproc_inner = INITPROC.inner_exclusive_access();
            for child in inner.children.iter() {
                child
                    .inner_exclusive_access()
                    .parent
                    .replace(INITPROC.pid.0);
                initproc_inner.children.push(child.clone());
            }

        } else {
            
            let w = &INITPROC;
            let _ = w;
        }


    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    // drop file descriptors
    inner.fd_table.clear();

    drop(inner);
    } 
    TID2TC.lock().remove(&tid);
    // 从进程中删除当前线程
    let mut tasks = process.tasks.lock();
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
static INITPROC_STR: &str = "ch6b_usertest";
lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: ProcessRef= Arc::new({
        let inode = open_file(INITPROC_STR, OpenFlags::O_RDONLY,0o777).unwrap();
        let v = inode.file().unwrap().read_all();
        ProcessControlBlock::new(v.as_slice(),"/".to_string())

    });

}

///Add init process to the manager
pub fn add_initproc() {
    PID2PC.lock().insert(INITPROC.pid.0, INITPROC.clone());
    // INITPROC.inner_exclusive_access().memory_set.activate();
    trace!("addInITPROC ok");
}
