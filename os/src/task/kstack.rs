use alloc::vec::Vec;
use core::{alloc::Layout, ptr::NonNull};
use lazy_init::LazyInit;
use spin::mutex::SpinMutex;

use crate::{
    config::{self, PAGE_SIZE},
    mm::VirtAddr,
};

pub struct TaskStack {
    ptr: NonNull<u8>,
    layout: Layout,
    is_init: bool,
}

// arch_boot

unsafe impl Send for StackPool {}
unsafe impl Sync for StackPool {}

impl TaskStack {
    // pub fn new_init() -> Self {
    //     let layout = Layout::from_size_align(config::TASK_STACK_SIZE, 16).unwrap();
    //         Self {
    //             ptr: ,
    //             layout,
    //             is_init: true,
    //         }

    // }

    pub fn alloc(size: usize) -> Self {
        // 向上对齐到整页
        let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let layout = Layout::from_size_align(aligned_size, PAGE_SIZE).unwrap();

        let ptr = unsafe { alloc::alloc::alloc(layout) };
        let ptr = NonNull::new(ptr).expect("Failed to allocate TaskStack");

        Self {
            ptr,
            layout,
            is_init: false,
        }
    }

    pub fn top(&self) -> VirtAddr {
        unsafe { core::mem::transmute(self.ptr.as_ptr().add(self.layout.size())) }
    }

    pub fn down(&self) -> VirtAddr {
        unsafe { core::mem::transmute(self.ptr.as_ptr()) }
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        if !self.is_init {
            unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), self.layout) }
        }
    }
}

// #[percpu::def_percpu]

static STACK_POOL: LazyInit<SpinMutex<StackPool>> = LazyInit::new();
pub fn init() {
    let mut stack_pool = StackPool::new();
    stack_pool.init();
    STACK_POOL.init_by(SpinMutex::new(stack_pool));
}
#[allow(dead_code)]
pub fn pick_current_stack() -> TaskStack {
    let mut stack_pool = STACK_POOL.lock();
    stack_pool.pick_current_stack()
}

pub fn current_stack_top() -> usize {
    let stack_pool = STACK_POOL.lock();

    // trace!("current ksp:{:#x}",stack_pool.current_stack().top().0);
    stack_pool.current_stack().top().0
}
pub fn current_stack_bottom() -> usize {
    let stack_pool = STACK_POOL.lock();

    // trace!("current ksp:{:#x}",stack_pool.current_stack().top().0);
    stack_pool.current_stack().down().0
}

#[allow(dead_code)]

pub fn put_prev_stack(kstack: TaskStack) {
    let mut stack_pool = STACK_POOL.lock();
    stack_pool.put_prev_stack(kstack)
}
pub fn _alloc_current_stack() {
    let mut stack_pool = STACK_POOL.lock();
    stack_pool.alloc();
}
/// A simple stack pool
#[allow(dead_code)]
pub(crate) struct StackPool {
    free_stacks: Vec<TaskStack>,
    current: Option<TaskStack>,
}

impl StackPool {
    /// Creates a new empty stack pool.
    pub const fn new() -> Self {
        Self {
            free_stacks: Vec::new(),
            current: None,
        }
    }

    pub fn init(&mut self) {
        self.current = Some(TaskStack::alloc(config::TASK_STACK_SIZE));
    }

    /// Alloc a free stack from the pool.
    pub fn alloc(&mut self) -> TaskStack {
        self.free_stacks.pop().unwrap_or_else(|| {
            let stack = TaskStack::alloc(config::TASK_STACK_SIZE);
            stack
        })
    }

    pub fn pick_current_stack(&mut self) -> TaskStack {
        let new_stack = self.alloc();
        self.current.replace(new_stack).unwrap()
    }

    pub fn current_stack(&self) -> &TaskStack {
        assert!(self.current.is_some());
        self.current.as_ref().unwrap()
    }

    pub fn put_prev_stack(&mut self, kstack: TaskStack) {
        assert!(self.current.is_some());
        let curr_stack = self.current.replace(kstack).unwrap();
        self.free_stacks.push(curr_stack);
    }
}
