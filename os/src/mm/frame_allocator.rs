//! Implementation of [`FrameAllocator`] which
//! controls all the frames in the operating system.
use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::mm::address::KernelAddr;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};
use core::panic;
use lazy_static::*;

/// tracker for physical page frame allocation and deallocation
pub struct FrameTracker {
    /// physical page number
     ppn: PhysPageNum,
}

impl FrameTracker {
    ///ppn
    pub fn ppn(&self) -> PhysPageNum {
        self.ppn
    }
    /// Create a new FrameTracker
    pub fn new(ppn: PhysPageNum) -> Self {
        // page cleaning
        let bytes_array = ppn.get_bytes_array();
        // if  ppn.0< 0x0812d2 ||ppn.0 >0x13FFFF  {
        //     panic!("FrameTracker:ppn={:#x} is not in the heap!", ppn.0);
        // }
         
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
        println!(
            "start frame={:#x},end frame={:#x},last {:#x} Physical Frames.",
            self.current,
            self.end,
            self.end - self.current
        );
        // trace!("last {} Physical Frames.", self.end - self.current);
        for i in self.recycled.iter() {
                  println!("{}",i);  
                }
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
       let res: Option<PhysPageNum>= if let Some(ppn) = self.recycled.pop() {
            // if ppn >= self.current {
            //     for i in self.recycled.iter() {
            //       println!("{}",i);  
            //     }
            //     panic!("Frame ppn={:#x} has not been allocated!", ppn);
            // }
            Some(ppn.into())
        } else if self.current == self.end {
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        };
        if res.unwrap().0>0x13ffff
        {
         if self.current == self.end {
            panic!("Frame is not enough!");//我草这里太玄学了无能为力也
                None
            } else {

                println!("[kernel] current ppn:{:#x}",self.current);
                self.current += 1;
                Some((self.current - 1).into())
            }
        }
        else
       { 
        res
       } 
        
    }
    fn dealloc(&mut self, ppn: PhysPageNum) {
        let ppn = ppn.0;
        // validity check
        if ppn >= self.current || self.recycled.iter().any(|&v| v == ppn) {
            panic!("Frame ppn={:#x} has not been allocated!", ppn);
        }
       
        self.recycled.push(ppn);
    }
}

type FrameAllocatorImpl = StackFrameAllocator;

lazy_static! {
    /// frame allocator instance through lazy_static!
    pub static ref FRAME_ALLOCATOR: UPSafeCell<FrameAllocatorImpl> =
        unsafe { UPSafeCell::new(FrameAllocatorImpl::new()) };
}
/// initiate the frame allocator using `ekernel` and `MEMORY_END`
pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    let l= PhysAddr::from(KernelAddr::from(ekernel as usize)).ceil();
    let r =
        PhysAddr::from(KernelAddr::from(MEMORY_END)).floor();
    FRAME_ALLOCATOR.exclusive_access().init(
        l,
        r,

        
    );
  
    info!("Frame_allocator l:{:#?},r:{:#?}",l,r);
}

/// Allocate a physical page frame in FrameTracker style
pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
}

/// Deallocate a physical page frame with a given ppn
pub fn frame_dealloc(ppn: PhysPageNum) {

   
   if ppn.0 > 0x100000000{
     panic!("Frame ppn={:#x} has not been allocated!", ppn.0);
   }
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
}

#[allow(unused)]
/// a simple test for frame allocator
pub fn frame_allocator_test() {
    let mut v: Vec<FrameTracker> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}
