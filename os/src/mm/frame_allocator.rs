//! Implementation of [`FrameAllocator`] which
//! controls all the frames in the operating system.
use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::mm::address::KernelAddr;
use crate::mm::{MPOL_BIND, MPOL_DEFAULT, MPOL_INTERLEAVE, MPOL_PREFERRED};
use crate::task::current_task_may_uninit;

use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::mutex::Mutex;
use core::fmt::{self, Debug, Formatter};
use core::panic;
use core::sync::atomic::AtomicUsize;
use lazy_static::*;

/// tracker for physical page frame allocation and deallocation
#[derive(Clone)]
pub struct FrameTracker {
    /// physical page number
     pub ppn: PhysPageNum,
}

impl FrameTracker {
    ///ppn
    pub fn ppn(&self) -> PhysPageNum {
        self.ppn
    }
    
    /// Create a new FrameTracker
     fn new(ppn: PhysPageNum) -> Self {
        // page cleaning
        let bytes_array = ppn.get_bytes_array();
        
         
        for i in bytes_array {
            *i = 0;

        }
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("FrameTracker:PPN={:#x}", self.ppn.0))
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        // if self.ppn.0==0x81901{
            
        //     println!("\ndrop pid:{}\n",current_task().unwrap().pid.0);
        //     bpoint();

        // }
        frame_dealloc(self.ppn);
    }
}
trait FrameAllocator {
    fn new() -> Self;
    fn alloc(&mut self,policy:usize) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}
/// an implementation for frame allocator
pub struct StackFrameAllocator {
    low_cursor: usize,
    high_cursor: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    pub fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        assert!(self.recycled.is_empty());
        self.low_cursor = l.0;
        self.high_cursor = r.0;
       
        // trace!("last {} Physical Frames.", self.end - self.current);
        // for i in self.recycled.iter() {
        //           println!("{}",i);  
        //         }
    }
    pub fn remaining_frames(&self) -> usize {
        // 剩余数量 = 回收队列中的数量 + 未分配的连续区域中的数量
        self.recycled.len() + (self.high_cursor - self.low_cursor)
    }
}
impl FrameAllocator for StackFrameAllocator {
    fn new() -> Self {
        Self {
            low_cursor: 0,
            high_cursor: 0,
            recycled: Vec::new(),
        }
    }
    fn alloc(&mut self,policy:usize) -> Option<PhysPageNum> {
        if let Some(ppn) = self.recycled.pop() {
            return Some(ppn.into());
        }
        if self.low_cursor >= self.high_cursor {
            error!("FrameAllocator: no more frames available, low_cursor:{:#x}, high_cursor:{:#x}", self.low_cursor, self.high_cursor);
            return None; // 物理内存耗尽
        }
        match policy {
            // 高地址优先策略
            MPOL_INTERLEAVE => {
                self.high_cursor -= 1;
                Some(self.high_cursor.into())
            }
            // 默认、绑定、首选策略都使用低地址优先
            MPOL_DEFAULT | MPOL_BIND | MPOL_PREFERRED | _ => {
                let ppn = self.low_cursor;
                self.low_cursor += 1;
                Some(ppn.into())
            }
        }
    }
    fn dealloc(&mut self, ppn: PhysPageNum) {
        
        let ppn = ppn.0;
        // validity check
        if ppn >= self.low_cursor || self.recycled.iter().any(|&v| v == ppn) {
        
            panic!("Frame ppn={:#x} has not been allocated!,current:{:#x}", ppn,self.low_cursor);
        }
       
        self.recycled.push(ppn);
        // println!("PPN:{:#x}",ppn );
    }
}

type FrameAllocatorImpl = StackFrameAllocator;

lazy_static! {
    /// frame allocator instance through lazy_static!
    pub static ref FRAME_ALLOCATOR: Mutex<FrameAllocatorImpl> =
         Mutex::new(FrameAllocatorImpl::new()) ;
}
/// initiate the frame allocator using `ekernel` and `MEMORY_END`
pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    let l= PhysAddr::from(KernelAddr::from(ekernel as usize)).ceil();
    let r =
        PhysAddr::from(KernelAddr::from(MEMORY_END)).floor();
    FRAME_ALLOCATOR.lock().init(
        l,
        r,

        
    );
  
    println!("Frame_allocator l:{:#x},r:{:#x}",l.0,r.0);
}

/// Allocate a physical page frame in FrameTracker style
pub fn frame_alloc() -> Option<Arc<FrameTracker>> {
    let policy = match current_task_may_uninit(){
        Some(task) =>task.get_noma_policy() ,
        None => 0,
    };
    FRAME_ALLOCATOR
        .lock()
        .alloc(policy)
        .map(FrameTracker::new)
        .map(Arc::new)
}

/// Allocate a continuous physical page frames in FrameTracker style
pub fn frame_alloc_continuous(num: usize) -> Option<Vec<Arc<FrameTracker>>> {
    let mut frames = Vec::new();
    for _ in 0..num {
        match frame_alloc() {
            Some(frame) => frames.push(frame),
            None => {
                // Rollback: deallocate already allocated frames
                for frame in frames {
                    frame_dealloc(frame.ppn());
                }
                return None;
            }
        }
    }
    Some(frames)
}
pub fn remaining_frames() -> usize {
    FRAME_ALLOCATOR.lock().remaining_frames()
}
/// Deallocate a physical page frame with a given ppn
pub fn frame_dealloc(ppn: PhysPageNum) {

   
   if ppn.0 > 0x100000000{
     panic!("Frame ppn={:#x} has not been allocated!", ppn.0);
   }
    FRAME_ALLOCATOR.lock().dealloc(ppn);
}

#[allow(unused)]
/// a simple test for frame allocator
pub fn frame_allocator_test() {
    let mut v: Vec<Arc<FrameTracker>> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:#?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:#?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}
