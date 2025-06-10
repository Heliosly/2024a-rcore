//! Implementation of [`FrameAllocator`] which
//! controls all the frames in the operating system.
use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::mm::address::KernelAddr;

use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::mutex::Mutex;
use core::fmt::{self, Debug, Formatter};
use core::panic;
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
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}
/// an implementation for frame allocator
pub struct StackFrameAllocator {
    current: usize,
    end: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    pub fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        assert!(self.recycled.is_empty());
        self.current = l.0;
        self.end = r.0;
       
        // trace!("last {} Physical Frames.", self.end - self.current);
        // for i in self.recycled.iter() {
        //           println!("{}",i);  
        //         }
    }
    pub fn remaining_frames(&self) -> usize {
        // 剩余数量 = 回收队列中的数量 + 未分配的连续区域中的数量
        self.recycled.len() + (self.end - self.current)
    }
}
impl FrameAllocator for StackFrameAllocator {
    fn new() -> Self {
        Self {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }
    fn alloc(&mut self) -> Option<PhysPageNum> {
      if let Some(ppn) = self.recycled.pop() {
            if ppn >= self.current {
             
                panic!("Frame alloc wrong ppn={:#x} !", ppn);
            }
            Some(ppn.into())
        } else if self.current == self.end {
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        }
        // if ret.unwrap().0 ==0x81901{
        //     print!("\nalloc pid:{}\n",current_task().unwrap().pid.0);
        //     bpoint();
        // }
        
        
    }
    fn dealloc(&mut self, ppn: PhysPageNum) {
        
        let ppn = ppn.0;
        // validity check
        if ppn >= self.current || self.recycled.iter().any(|&v| v == ppn) {
        
            panic!("Frame ppn={:#x} has not been allocated!,current:{:#x}", ppn,self.current);
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
    FRAME_ALLOCATOR
        .lock()
        .alloc()
        .map(FrameTracker::new)
        .map(Arc::new)
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
