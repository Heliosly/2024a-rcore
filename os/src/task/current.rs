use core::{mem::ManuallyDrop, ops::Deref, task::Waker};

use alloc::sync::Arc;

use super::{schedule::{Task, TaskRef},  TaskStatus};


#[inline]
fn local_irq_save_and_disable() -> usize {
    const SIE_BIT: usize = 1 << 1;
    let flags: usize;
    // clear the `SIE` bit, and return the old CSR
    unsafe { core::arch::asm!("csrrc {}, sstatus, {}", out(reg) flags, const SIE_BIT) };
    flags & SIE_BIT
}

fn local_irq_restore(flags: usize) {
    // restore the `SIE` bit
    unsafe { core::arch::asm!("csrrs x0, sstatus, {}", in(reg) flags) };
}
#[link_section = ".percpu"]
static mut __PERCPU_CURRENT_TASK_PTR: usize = 0;

/// Wrapper struct for the per-CPU data [stringify! (CURRENT_TASK_PTR)]
struct CURRENT_TASK_PTR_WRAPPER {}

#[allow(unused)]
static CURRENT_TASK_PTR: CURRENT_TASK_PTR_WRAPPER = CURRENT_TASK_PTR_WRAPPER {};

#[allow(dead_code)]
impl CURRENT_TASK_PTR_WRAPPER {
    /// Returns the offset relative to the per-CPU data area base on the current CPU.
    fn offset(&self) -> usize {
        let value: usize;
        unsafe {
          
            core::arch::asm!(
                "lui {0}, %hi({VAR})",
                "addi {0}, {0}, %lo({VAR})",
                out(reg) value,
                VAR = sym __PERCPU_CURRENT_TASK_PTR,
            );
        }
        value
    }
    #[inline]
    /// Returns the raw pointer of this per-CPU data on the current CPU.
    ///
    /// # Safety
    ///
    /// Caller must ensure that preemption is disabled on the current CPU.
    pub unsafe fn current_ptr(&self) -> *const usize {
             let base:usize;
                core::arch::asm! ("mv {}, gp", out(reg) base);
                (base + self.offset()) as *const usize
          
    }

    #[inline]
    /// Returns the reference of the per-CPU data on the current CPU.
    ///
    /// # Safety
    ///
    /// Caller must ensure that preemption is disabled on the current CPU.
    pub unsafe fn current_ref_raw(&self) -> &usize {
        &*self.current_ptr()
    }

    #[inline]
    /// Returns the mutable reference of the per-CPU data on the current CPU.
    ///
    /// # Safety
    ///
    /// Caller must ensure that preemption is disabled on the current CPU.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn current_ref_mut_raw(&self) -> &mut usize {
        &mut *(self.current_ptr() as *mut usize)
    }

    /// Manipulate the per-CPU data on the current CPU in the given closure.
    ///
    /// Preemption will be disabled during the call.
    pub fn with_current<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut usize) -> T,
    {
        f(unsafe { self.current_ref_mut_raw() })
    }

    #[inline]
    /// Returns the value of the per-CPU data on the current CPU.
    ///
    /// # Safety
    ///
    /// Caller must ensure that preemption is disabled on the current CPU.
    pub unsafe fn read_current_raw(&self) -> usize {
        let ret: usize;
        core::arch::asm!(
            "la   {tmp}, {sym}",
            "ld   {ret}, 0({tmp})",
            tmp = out(reg) _,
            ret = out(reg) ret,
            sym = sym __PERCPU_CURRENT_TASK_PTR,
        );
        ret
    }
    

    #[inline]
    /// Set the value of the per-CPU data on the current CPU.
    ///
    /// # Safety
    ///
    /// Caller must ensure that preemption is disabled on the current CPU.
    pub unsafe fn write_current_raw(&self, val: usize) {
        core::arch::asm!(
           
            "la   {tmp}, {sym}",       // 用占位符 {tmp}
            "sd   {val}, 0({tmp})",    // 存 val 到 *tmp
            tmp = out(reg) _,
            val = in(reg) val,
            sym = sym __PERCPU_CURRENT_TASK_PTR,
        );
    }

    /// Returns the value of the per-CPU data on the current CPU. Preemption will
    /// be disabled during the call.
    pub fn read_current(&self) -> usize {
        unsafe { self.read_current_raw() }
    }

    /// Set the value of the per-CPU data on the current CPU.
    /// Preemption will be disabled during the call.
    pub fn write_current(&self, val: usize) {
        unsafe { self.write_current_raw(val) }
    }
}

/// Gets the pointer to the current task with preemption-safety.
///
/// Preemption may be enabled when calling this function. This function will
/// guarantee the correctness even the current task is preempted.
#[inline]
pub fn current_task_ptr<T>() -> *const T {
  
  
    unsafe {
        // on RISC-V, reading `CURRENT_TASK_PTR` requires multiple instruction, so we disable local IRQs.
        let flags = local_irq_save_and_disable();
        let ans = CURRENT_TASK_PTR.read_current_raw();
        local_irq_restore(flags);
        ans as _
    }
   
}
/// Sets the pointer to the current task with preemption-safety.
///
/// Preemption may be enabled when calling this function. This function will
/// guarantee the correctness even the current task is preempted.
///
/// # Safety
///
/// The given `ptr` must be pointed to a valid task structure.
#[inline]
pub unsafe fn set_current_task_ptr<T>(ptr: *const T) {
 
        let flags = local_irq_save_and_disable();
        CURRENT_TASK_PTR.write_current_raw(ptr as usize);
        local_irq_restore(flags)
   
}

/// A wrapper of [`TaskRef`] as the current task.
pub struct CurrentTask(pub ManuallyDrop<TaskRef>);

impl CurrentTask {
    pub fn try_get() -> Option<Self> {
        let ptr:*const Task = current_task_ptr();
        if !ptr.is_null() {
            Some(Self(unsafe { ManuallyDrop::new(TaskRef::from_raw(ptr)) }))
        } else {
            None
        }
    }

    pub fn get() -> Self {
        Self::try_get().expect("current task is uninitialized")
    }

    /// Converts [`CurrentTask`] to [`TaskRef`].
    pub fn as_task_ref(&self) -> &TaskRef {
        &self.0
    }

    pub fn clone(&self) -> TaskRef {
        self.0.deref().clone()
    }

    pub fn ptr_eq(&self, other: &TaskRef) -> bool {
        Arc::ptr_eq(&self.0, other)
    }

    pub unsafe fn init_current(init_task: TaskRef) {
        ///MUST1
        init_task.set_state(TaskStatus::Running);
        let ptr = Arc::into_raw(init_task);
        set_current_task_ptr(ptr);
    }

    pub fn clean_current() {
        let curr = Self::get();
        let Self(arc) = curr;
        ManuallyDrop::into_inner(arc); // `call Arc::drop()` to decrease prev task reference count.
        unsafe { set_current_task_ptr(0 as *const Task) };
    }

    pub fn clean_current_without_drop() -> *const Task {
        let ptr: *const Task = current_task_ptr();
        unsafe { set_current_task_ptr(0 as *const Task) };
        ptr
    }

    pub fn waker(&self) -> Waker {
        crate::task::waker::waker_from_task(current_task_ptr() as _)
    }
}

impl Deref for CurrentTask {
    type Target = Task;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub fn current_task_may_uninit() -> Option<CurrentTask> {
    CurrentTask::try_get()
}

pub fn current_task() -> CurrentTask {
    CurrentTask::get()
}

pub fn current_task_token() -> usize {
    current_task().get_user_token()
}