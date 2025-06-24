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
pub mod auxv;
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
use riscv::addr::VirtAddr;
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
use crate::mm::{get_target_ref, put_data};
use crate::sync::futex::GLOBAL_FUTEX_SYSTEM;
use crate::syscall::flags::{FUTEX_OWNER_DIED, FUTEX_TID_MASK, FUTEX_WAITERS};
use crate::task::task::TaskControlBlock;
use crate::utils::error::GeneralRet;
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
// 在 TaskControlBlock 中

/// 终止一个进程及其所有线程，并回收资源。
/// 这是 exit_group(2) 系统调用的内核实现。
pub async fn exit_proc(exit_code: i32) {
    let process = current_process();
    let pid = process.get_pid();
    debug!("[kernel] exit_group pid {}, exit_code: {}", pid, exit_code);

    // --- 第 1 步：标记所有线程为 Zombie，并准备清理 ---
    // 复制一份线程列表，因为我们不能在持有锁的同时执行可能休眠的 clear_child_tid
    let tasks_to_terminate= {
        let tasks_guard = process.tasks.lock().await;
        tasks_guard.clone()
    };
    process.set_exit_code(exit_code);
    *process.state.lock().await=TaskStatus::Zombie;
    process.wake_all_waiters().await;
    for thread in tasks_to_terminate.iter() {
        
        // 清理 futexes 。这可能会唤醒其他进程。
        if let Some(ptr)=thread.child_tid_ptr(){

        if process.manual_alloc_type_for_lazy(ptr as *const u32).await.is_ok(){

           let _ = thread.clear_child_tid().await;
        }

        }
    }
    
    // --- 第 2 步：为子进程重新指定父进程 (reparenting) ---
    // 只有非 init 进程需要 reparent
    if pid != INITPROC.get_pid() {
        let mut children = process.children.lock().await;
        let mut init_children = INITPROC.children.lock().await;
        for child in children.drain(..) { // 使用 drain 高效移动元素
            child.set_parent(INITPROC.pid.0);
            init_children.push(child);
        }
    }

    // --- 第 3 步：回收进程级资源 ---
    process.memory_set.lock().await.recycle_data_pages().await.unwrap();
    process.fd_table.lock().await.table.clear();

    // --- 第 4 步：从全局数据结构中移除所有线程 ---
    {
        // 再次获取 tasks 锁，清空它
        let mut tasks_guard = process.tasks.lock().await;
        for thread in tasks_guard.iter() {
            let tid = thread.id.as_usize();
            TID2TC.lock().remove(&tid);
        }
        tasks_guard.clear();
    }
    
}
/// 终止当前线程。如果这是进程中的最后一个线程，则等同于 exit_group。
/// 这是 exit(2) 系统调用的内核实现。
pub async fn exit_current(exit_code: i32) {
    let task = current_task();
    let process = current_process();
    debug!("[kernel] exit_current tid {}, exit_code: {}", task.id(), exit_code);
    // 检查是否是进程中的最后一个线程
    let is_lead = task.is_leader();

    if is_lead {
        // 如果是最后一个线程，行为等同于 exit_group
        // 直接调用新的 exit_group 函数即可
        exit_proc(exit_code).await;
    } else {
        // --- 只退出当前线程 ---
        let tid = task.id.as_usize();
        
        // 1. 标记为 Zombie 并清理
        task.set_state(TaskStatus::Zombie);
        task.set_exit_code(exit_code as isize);
        let _ = task.clear_child_tid().await;
        exit_robust_list_cleanup(task.robust_list.lock().await.head).await.unwrap();

        // 2. 从进程的线程列表中移除自己
        {
            let mut tasks_guard = process.tasks.lock().await;
            tasks_guard.retain(|t| t.id.as_usize() != tid);
            process.wakers.lock().await.remove(&tid);
        }
        
        // 3. 从全局 TID 映射中移除自己
        TID2TC.lock().remove(&tid);

        // drop(task) 将由 Arc 自动处理
        // 进程的其他资源 (内存, FD) 保持不变，因为其他线程还在运行。
    }
}

#[repr(C)]
    #[derive(Debug, Clone, Copy)] // 加上 Clone, Copy
    struct UserRobustListHead {
        list: UserRobustList,
        futex_offset: isize, // 【建议】改成 isize 更健壮
        list_op_pending: usize, 
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct UserRobustList {
        next: usize, 
    }

async fn exit_robust_list_cleanup(head_ptr: usize) -> GeneralRet {



    //println!("[ROBUST_DEBUG] >> Entering exit_robust_list_cleanup with head_ptr: {:#x}", head_ptr);

    let mut current_head_ptr = head_ptr as *const UserRobustListHead;
    let current_task_tid = current_task().get_tid();
    let token = current_token().await;
    const MAX_ROBUST_LIST_NODES: usize = 100;

    for i in 0..MAX_ROBUST_LIST_NODES {
        if current_head_ptr.is_null() {
            //println!("[ROBUST_DEBUG] Head pointer is NULL. Cleanup finished on iteration {}.", i);
            break;
        }

        // 1. 从用户空间读取 robust_list_head 结构体
        //println!("[ROBUST_DEBUG] Attempting to read UserRobustListHead from user address: {:?}", current_head_ptr);
        let head_data: UserRobustListHead = match get_target_ref(token, current_head_ptr) { // 使用 get_target_data 获取拷贝
            Ok(data) => *data,
            Err(_) => {
                //println!("[ROBUST_DEBUG] !!! ERROR: Failed to read UserRobustListHead at {:?}", current_head_ptr);
                break;
            }
        };
        // 打印读取到的完整 head_data
        //println!("[ROBUST_DEBUG] Successfully read head_data: {:?}", head_data);


        // 2. 处理 list_op_pending
        if head_data.list_op_pending != 0 {
            let pending_lock_addr = head_data.list_op_pending;
            
            //println!("[ROBUST_DEBUG] Found pending operation on lock at addr: {:#x}", pending_lock_addr);
        
            let futex_val_result = get_target_ref(token, pending_lock_addr as *const u32);
        
            if let Ok(futex_val) = futex_val_result {
                let futex_val = *futex_val;
                //println!("[ROBUST_DEBUG] Pending lock value is: {:#x}", futex_val);
                if (futex_val & FUTEX_TID_MASK) == current_task_tid as u32 {
                    //println!("[ROBUST_DEBUG] Pending lock owner matches current TID ({}). Cleaning up.", current_task_tid);
        
                    let new_futex_val = (futex_val & FUTEX_WAITERS) | FUTEX_OWNER_DIED;
                    //println!("[ROBUST_DEBUG] New futex value will be: {:#x}", new_futex_val);
        
                    if put_data(token, pending_lock_addr as *mut u32, new_futex_val).is_ok() {
                        if (futex_val & FUTEX_WAITERS) != 0 {
                            //println!("[ROBUST_DEBUG] Waking waiters for pending lock at {:#x}", pending_lock_addr);
                            let mut futex_guard = GLOBAL_FUTEX_SYSTEM.lock();
                        if let Some(wait_queue) = futex_guard.get_mut(&(token, pending_lock_addr)) {
                            wait_queue.wake_matching_waiters(1, u32::MAX);
                            if wait_queue.is_empty() {
                                futex_guard.remove(&(token, pending_lock_addr));
                            }
                        }
                        }
                    } else {
                         //println!("[ROBUST_DEBUG] !!! ERROR: Failed to write new value to pending lock at {:#x}", pending_lock_addr);
                    }
                } else {
                    //println!("[ROBUST_DEBUG] Pending lock owner ({}) does not match current TID ({}). Skipping.", futex_val & FUTEX_TID_MASK, current_task_tid);
                }
            } else {
                //println!("[ROBUST_DEBUG] !!! ERROR: Failed to read pending lock value at {:#x}", pending_lock_addr);
            }
        }

        // 3. 遍历由 head_data.list.next 开始的锁链表
        let mut current_node_ptr = head_data.list.next;
        //println!("[ROBUST_DEBUG] Starting lock list traversal from node_ptr: {:#x}. Head is: {:#x}", current_node_ptr, head_ptr);

        while current_node_ptr != head_ptr {
            if current_node_ptr == 0 {
                //println!("[ROBUST_DEBUG] Encountered NULL node_ptr in list. Breaking loop.");
                break;
            }

            //println!("[ROBUST_DEBUG] --- Processing node at: {:#x} ---", current_node_ptr);

            // a. 读取 list 节点
            let node_data_result = get_target_ref(token, current_node_ptr as *const UserRobustList);
            let node_data: UserRobustList = match node_data_result {
                Ok(data) => *data,
                Err(_) => {
                    //println!("[ROBUST_DEBUG] !!! ERROR: Failed to read UserRobustList node at {:#x}", current_node_ptr);
                    break;
                }
            };
            //println!("[ROBUST_DEBUG] Node data: {:?}", node_data);
            

            // b. 计算锁的地址
            //    为了安全，我们使用 wrapping_sub 来防止 panic，但如果发生回环，说明有逻辑错误
            let lock_addr = (current_node_ptr as isize).wrapping_add(head_data.futex_offset) as usize;
            //println!("[ROBUST_DEBUG] Calculated lock_addr: {:#x}", lock_addr);


            // c. 读取并检查锁的状态
            let futex_val_result = get_target_ref(token, lock_addr as *const u32);
            if let Ok(futex_val) = futex_val_result {
                let futex_val = *futex_val;
                //println!("[ROBUST_DEBUG] Lock value at {:#x} is: {:#x}", lock_addr, futex_val);

                // d. 检查所有者是否是当前线程
                if (futex_val & FUTEX_TID_MASK) == current_task_tid as u32 {
                    //println!("[ROBUST_DEBUG] Lock owner matches current TID ({}). Cleaning up.", current_task_tid);
                    let new_futex_val = (futex_val & !FUTEX_TID_MASK) | FUTEX_OWNER_DIED;
                    //println!("[ROBUST_DEBUG] New futex value will be: {:#x}", new_futex_val);
                    
                    if put_data(token, lock_addr as *mut u32, new_futex_val).is_ok() {
                        if (futex_val & FUTEX_WAITERS) != 0 {
                            //println!("[ROBUST_DEBUG] Waking waiters for lock at {:#x}", lock_addr);
                            let mut futex_guard = GLOBAL_FUTEX_SYSTEM.lock();
                        if let Some(wait_queue) = futex_guard.get_mut(&(token, lock_addr)) {
                            wait_queue.wake_matching_waiters(1, u32::MAX);
                            if wait_queue.is_empty() {
                                futex_guard.remove(&(token, lock_addr));
                            }
                        }
                        }
                    } else {
                        //println!("[ROBUST_DEBUG] !!! ERROR: Failed to write new value to lock at {:#x}", lock_addr);
                    }
                } else {
                     //println!("[ROBUST_DEBUG] Lock owner ({}) does not match current TID ({}). Skipping.", futex_val & FUTEX_TID_MASK, current_task_tid);
                }
            } else {
                 //println!("[ROBUST_DEBUG] !!! ERROR: Failed to read lock value at {:#x}", lock_addr);
            }
            
            // g. 前进到下一个节点
            current_node_ptr = node_data.next;
        }
        
        //println!("[ROBUST_DEBUG] Finished list traversal (or loop broke).");
        break; 
    }

    //println!("[ROBUST_DEBUG] << Leaving exit_robust_list_cleanup.");
    Ok(())
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