//! Memory management implementation
//!
//! SV39 page-based virtual-memory architecture for RV64 systems, and
//! everything about memory management, like frame allocator, page table,
//! map area and memory set, is implemented here.
//!
//! Every task or process has a memory_set to control its virtual memory.

mod address;
pub(crate) mod frame_allocator;
pub mod heap_allocator;
mod memory_set;
mod page_table;

use core::arch::asm;

pub use page_table::put_data;
pub use address::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum,KernelAddr,VPNRange};
pub use frame_allocator::{frame_alloc, frame_dealloc, FrameTracker};
pub use memory_set::remap_test;
pub use memory_set::{kernel_token, MapPermission, MemorySet, KERNEL_SPACE,MapAreaType,VmAreaTree,MapArea,MapType};
use page_table::PTEFlags;
pub use page_table::{
    translated_byte_buffer, translated_ref, translated_refmut, translated_str, PageTable,get_target_ref_mut,
    PageTableEntry, UserBuffer, UserBufferIterator,fill_str
};
/// initiate heap allocator, frame allocator and kernel space
pub fn init() {
   
    heap_allocator::init_heap();
    frame_allocator::init_frame_allocator();
    KERNEL_SPACE.exclusive_access().activate();
}

pub  fn flush_tlb(){
    unsafe  {
    asm!("sfence.vma");
    }
}