use core::{mem::ManuallyDrop, ops::Deref, task::Waker};

use alloc::sync::Arc;

use super::{
    schedule::{Task, TaskRef},
    ProcessRef, TaskStatus, PID2PC,
};

#[inline]
fn local_irq_save_and_disable() -> usize {
    let mut flags: usize;
    const SIE_BIT: usize = 1 << 1;

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    unsafe {
        // clear the `SIE` bit, and return the old CSR
        core::arch::asm!("csrrc {}, sstatus, {}", out(reg) flags, const SIE_BIT);
    }

    #[cfg(target_arch = "loongarch64")]
    unsafe {
        // LoongArch64: 读取并清除 CRMD 的 IE 位 (bit 2)
        let old_flags: usize;
        core::arch::asm!(
            "csrrd {old}, 0x0",         // 读取 CSR_CRMD
            "li.d $t0, 0x4",            // IE bit mask
            "andn {new}, {old}, $t0",   // 清除 IE 位
            "csrwr {new}, 0x0",         // 写回 CSR_CRMD
            old = out(reg) old_flags,
            new = out(reg) flags,
        );
        flags = old_flags; // 返回原始值
    }

    flags & SIE_BIT
}

fn local_irq_restore(flags: usize) {
    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    unsafe {
        // restore the `SIE` bit
        core::arch::asm!("csrrs x0, sstatus, {}", in(reg) flags);
    }

    #[cfg(target_arch = "loongarch64")]
    unsafe {
        // LoongArch64: 恢复 CRMD 的 IE 位
        if flags & 0x4 != 0 {
            core::arch::asm!(
                "csrrd $t0, 0x0",    // 读取 CSR_CRMD
                "ori $t0, $t0, 0x4", // 设置 IE 位
                "csrwr $t0, 0x0",    // 写回 CSR_CRMD
            );
        }
    }
}

#[link_section = ".percpu"]
static mut __PERCPU_CURRENT_TASK_PTR: usize = 0;

#[allow(non_camel_case_types)]
/// Wrapper struct for the per-CPU data [stringify! (CURRENT_TASK_PTR)]
struct CURRENT_TASK_PTR_WRAPPER {}

static CURRENT_TASK_PTR: CURRENT_TASK_PTR_WRAPPER = CURRENT_TASK_PTR_WRAPPER {};

impl CURRENT_TASK_PTR_WRAPPER {
    /// Returns the offset relative to the per-CPU data area base on the current CPU.
    fn offset(&self) -> usize {
        let value: usize;
        unsafe {
            #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
            core::arch::asm!(
                "lui {0}, %hi({VAR})",
                "addi {0}, {0}, %lo({VAR})",
                out(reg) value,
                VAR = sym __PERCPU_CURRENT_TASK_PTR,
            );

            #[cfg(target_arch = "loongarch64")]
            core::arch::asm!(
                "la.global {0}, {VAR}",
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
        let base: usize;
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        core::arch::asm!("mv {}, gp", out(reg) base);

        #[cfg(target_arch = "loongarch64")]
        core::arch::asm!("or {}, $r21, $zero", out(reg) base); // $r21 是 LoongArch64 的 GP 寄存器

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
        
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        core::arch::asm!(
            "la   {tmp}, {sym}",
            "ld   {ret}, 0({tmp})",
            tmp = out(reg) _,
            ret = out(reg) ret,
            sym = sym __PERCPU_CURRENT_TASK_PTR,
        );
        
        #[cfg(target_arch = "loongarch64")]
        core::arch::asm!(
            "la.global {tmp}, {sym}",
            "ld.d {ret}, {tmp}, 0",      // LoongArch64 语法：ld.d dst, base, offset
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
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        core::arch::asm!(
            "la   {tmp}, {sym}",       // 用占位符 {tmp}
            "sd   {val}, 0({tmp})",    // 存 val 到 *tmp
            tmp = out(reg) _,
            val = in(reg) val,
            sym = sym __PERCPU_CURRENT_TASK_PTR,
        );
        
        #[cfg(target_arch = "loongarch64")]
        core::arch::asm!(
            "la.global {tmp}, {sym}",
            "st.d {val}, {tmp}, 0",    // LoongArch64 语法：st.d src, base, offset
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
        let ptr: *const Task = current_task_ptr();
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
        //MUST1
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

pub fn current_process() -> ProcessRef {
    current_task().get_process().unwrap()
}

pub async fn current_token() -> usize {
    let current_task = current_task();

    PID2PC
        .lock()
        .get(&current_task.get_pid())
        .unwrap()
        .get_user_token()
        .await
}

pub fn current_task_id() -> usize {
    current_task().id()
}

pub fn current_task_id_may_uninit() -> usize {
    match CurrentTask::try_get() {
        Some(curr) => curr.id(),

        None => 1,
    }
}

// #[link_section = ".percpu"]
// static mut __PERCPU_CURRENT_PROCESS_PTR: usize = 0;

// #[allow(non_camel_case_types)]
// /// Wrapper struct for the per-CPU data [stringify! (CURRENT_ProcessControlBlock_PTR)]
// struct CURRENT_PROCESS_PTR_WRAPPER {}

// static CURRENT_PROCESS_PTR:CURRENT_PROCESS_PTR_WRAPPER= CURRENT_PROCESS_PTR_WRAPPER {};

// impl CURRENT_PROCESS_PTR_WRAPPER {
//     /// Returns the offset relative to the per-CPU data area base on the current CPU.
//     fn offset(&self) -> usize {
//         let value: usize;
//         unsafe {
//             core::arch::asm!(
//                 "lui {0}, %hi({VAR})",
//                 "addi {0}, {0}, %lo({VAR})",
//                 out(reg) value,
//                 VAR = sym __PERCPU_CURRENT_PROCESS_PTR,
//             );
//         }
//         value
//     }
//     #[inline]
//     /// Returns the raw pointer of this per-CPU data on the current CPU.
//     ///
//     /// # Safety
//     ///
//     /// Caller must ensure that preemption is disabled on the current CPU.
//     pub unsafe fn current_ptr(&self) -> *const usize {

//             let base: usize;

//                 core::arch::asm! ("mv {}, gp", out(reg) base);
//                 (base + self.offset()) as *const usize

//     }

//     #[inline]
//     /// Returns the reference of the per-CPU data on the current CPU.
//     ///
//     /// # Safety
//     ///
//     /// Caller must ensure that preemption is disabled on the current CPU.
//     pub unsafe fn current_ref_raw(&self) -> &usize {
//         &*self.current_ptr()
//     }

//     #[inline]
//     /// Returns the mutable reference of the per-CPU data on the current CPU.
//     ///
//     /// # Safety
//     ///
//     /// Caller must ensure that preemption is disabled on the current CPU.
//     #[allow(clippy::mut_from_ref)]
//     pub unsafe fn current_ref_mut_raw(&self) -> &mut usize {
//         &mut *(self.current_ptr() as *mut usize)
//     }

//     /// Manipulate the per-CPU data on the current CPU in the given closure.
//     ///
//     /// Preemption will be disabled during the call.
//     pub fn with_current<F, T>(&self, f: F) -> T
//     where
//         F: FnOnce(&mut usize) -> T,
//     {
//         f(unsafe { self.current_ref_mut_raw() })
//     }

//     #[inline]
//     /// Returns the value of the per-CPU data on the current CPU.
//     ///
//     /// # Safety
//     ///
//     /// Caller must ensure that preemption is disabled on the current CPU.
//     pub unsafe fn read_current_raw(&self) -> usize {
//         let ret: usize;
//         core::arch::asm!(
//             // la 会根据符号距离自动展开为 auipc/addi 或更复杂序列
//             "la   {tmp}, {sym}",
//             "ld   {ret}, 0({tmp})",
//             tmp = out(reg) _,
//             ret = out(reg) ret,
//             sym = sym __PERCPU_CURRENT_PROCESS_PTR,
//         );
//         ret
//     }
//     #[inline]
//     /// Set the value of the per-CPU data on the current CPU.
//     ///
//     /// # Safety
//     ///
//     /// Caller must ensure that preemption is disabled on the current CPU.
//     pub unsafe fn write_current_raw(&self, val: usize) {
//         core::arch::asm!(
//             "la   {tmp}, {sym}",
//             "sd   {val}, 0({tmp})",
//             tmp = out(reg) _,
//             val = in(reg) val,
//             sym = sym __PERCPU_CURRENT_PROCESS_PTR,
//         );
//     }
//     /// Returns the value of the per-CPU data on the current CPU. Preemption will
//     /// be disabled during the call.
//     pub fn read_current(&self) -> usize {
//         unsafe { self.read_current_raw() }
//     }

//     /// Set the value of the per-CPU data on the current CPU.
//     /// Preemption will be disabled during the call.
//     pub fn write_current(&self, val: usize) {
//         unsafe { self.write_current_raw(val) }
//     }
// }

// /// Gets the pointer to the current task with preemption-safety.
// ///
// /// Preemption may be enabled when calling this function. This function will
// /// guarantee the correctness even the current task is preempted.
// #[inline]
// pub fn current_process_ptr<T>() -> *const T {
//     unsafe {
//         // on RISC-V, reading `CURRENT_process_PTR` requires multiple instruction, so we disable local IRQs.
//         let flags = local_irq_save_and_disable();
//         let ans = CURRENT_PROCESS_PTR.read_current_raw();
//         local_irq_restore(flags);
//         ans as _
//     }
// }
// /// Sets the pointer to the current task with preemption-safety.
// ///
// /// Preemption may be enabled when calling this function. This function will
// /// guarantee the correctness even the current task is preempted.
// ///
// /// # Safety
// ///
// /// The given `ptr` must be pointed to a valid task structure.
// #[inline]
// pub unsafe fn set_current_process_ptr<T>(ptr: *const T) {

//         let flags = local_irq_save_and_disable();
//         CURRENT_PROCESS_PTR.write_current_raw(ptr as usize);
//         local_irq_restore(flags)

// }

// /// A wrapper of [`TaskRef`] as the current task.
// pub struct CurrentProcess(ManuallyDrop<ProcessRef>);

// impl CurrentProcess {
//     pub(crate) fn try_get() -> Option<Self> {
//         let ptr: *const ProcessControlBlock = current_process_ptr();
//         if !ptr.is_null() {
//             Some(Self(unsafe {
//                 ManuallyDrop::new(ProcessRef::from_raw(ptr))
//             }))
//         } else {
//             None
//         }
//     }

//     pub fn get() -> Self {
//         Self::try_get().expect("current process is uninitialized")
//     }

//     #[allow(unused)]
//     /// Converts [`CurrentTask`] to [`TaskRef`].
//     pub fn as_process_ref(&self) -> &ProcessRef {
//         &self.0
//     }

//     #[allow(unused)]
//     pub fn clone(&self) -> ProcessRef {
//         self.0.deref().clone()
//     }

//     #[allow(unused)]
//     pub fn ptr_eq(&self, other: &ProcessRef) -> bool {
//         Arc::ptr_eq(&self.0, other)
//     }

//     pub unsafe fn init_current(process: ProcessRef) {
//         let ptr = Arc::into_raw(process);
//         set_current_process_ptr(ptr);
//     }

//     pub fn clean_current() {
//         let curr = Self::get();
//         let Self(arc) = curr;
//         ManuallyDrop::into_inner(arc); // `call Arc::drop()` to decrease prev task reference count.
//         unsafe { set_current_process_ptr(0 as *const ProcessControlBlock) };
//     }

//     pub fn clean_current_without_drop() {
//         unsafe { set_current_process_ptr(0 as *const ProcessControlBlock) };
//     }
// }

// impl Deref for CurrentProcess {
//     type Target = ProcessControlBlock;
//     fn deref(&self) -> &Self::Target {
//         self.0.deref()
//     }
// }
