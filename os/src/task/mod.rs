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

mod id;
mod processor;
mod waker;
mod kstack;
mod schedule;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
mod yieldfut;
// pub use manager::get_task_count;
pub use processor::run_task2;
pub use kstack::TaskStack;
use processor::PROCESSOR;
use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;
use lazy_static::*;
// pub use manager::{fetch_task, TaskManager,pick_next_task};
pub use task::{TaskControlBlock, TaskStatus};
pub use id::RecycleAllocator;
pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
// pub use manager::add_task;
pub use processor::{
    current_task, current_user_token, run_tasks,  take_current_task,
    Processor,
};
pub use core::mem::ManuallyDrop;
pub use processor::init;
pub use yieldfut::yield_now;
pub use processor::current_task_trapctx_ptr;
pub use schedule::{add_task,pick_next_task,put_prev_task,task_tick,set_priority};
use core::task::Waker;
use core::ops::Deref;
pub struct CurrentTask(ManuallyDrop<Arc<TaskControlBlock>>);
impl CurrentTask {
    pub unsafe fn init_current(init_task:Arc<TaskControlBlock> ) {
        init_task.set_state(TaskStatus::Running);
        PROCESSOR.exclusive_access().set_current(init_task);

    }
    /// get a new waker of the CurrentTask;
    pub fn waker(&self) -> Waker {
        if let Some(task_arc) = current_task() {
            let raw: *const TaskControlBlock = Arc::as_ptr(&task_arc);
            // raw 现在就是指向 TaskControlBlock 的裸指针
            // 此时不影响 Arc 的引用计数

        waker::waker_from_task(raw as _)
        }
        else{
            panic!("current task is uninitialized");
        }
    }
    // pub fn try_get() -> Option<Self> {
    //     let ptr: *const super::Task = current_task_ptr();
    //     if !ptr.is_null() {
    //         Some(Self(unsafe { ManuallyDrop::new(TaskRef::from_raw(ptr)) }))
    //     } else {
    //         None
    //     }
    // }

    pub fn from(task: Arc<TaskControlBlock>) -> Self {
        CurrentTask(ManuallyDrop::new(task))
    }
    pub fn get() -> Self {
       CurrentTask(ManuallyDrop::new(
            current_task().expect("current task is uninitialized")
        ))
    }
    /// Converts [`CurrentTask`] to [`TaskRef`].
    pub fn as_task_ref(&self) -> &Arc<TaskControlBlock> {
        &self.0
    }
    pub fn clean_current_without_drop() -> Option<Arc<TaskControlBlock>> {
        take_current_task()
    }

    pub fn clone(&self) -> Arc<TaskControlBlock> {
        self.0.deref().clone()
    }

    pub fn ptr_eq(&self, other:  &Arc<TaskControlBlock>) -> bool {
        Arc::ptr_eq(&self.0, other)
    }
    pub fn clean_current(&mut self) {
        take_current_task();
       
        self.drop();
    }
    pub fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.0);
        }
    }
}

impl Deref for CurrentTask {
    type Target = TaskControlBlock;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

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
    let task = current_task().unwrap();

    println!("exit pid {},exit code:{}",task.pid.0,exit_code);
    // if pid == IDLE_PID {
    //     println!(
    //         "[kernel] Idle process exit with exit_code {} ...",
    //         exit_code
    //     );
    //     panic!("All applications completed!");
    // }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
       // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    if task.pid.0 !=0
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    else{
        let w = &INITPROC;
        let _ = w;
    }
    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    // drop file descriptors
    inner.fd_table.clear();
    inner.set_state(TaskStatus::Zombie);
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);

}

lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new({
        let inode = open_file("ch6b_initproc", OpenFlags::O_RDONLY,0o777).unwrap();
        let v = inode.file().unwrap().read_all();
        TaskControlBlock::new(v.as_slice())
    });
}

///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
    trace!("addInITPROC ok");
}
