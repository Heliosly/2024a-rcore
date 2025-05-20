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
pub mod page_table;
use core::arch::asm;

use alloc::sync::Arc;
use lazy_init::LazyInit;
pub use page_table::put_data;
pub use address::{PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum,KernelAddr,VPNRange};
pub use frame_allocator::{frame_alloc, frame_dealloc, FrameTracker};
pub use memory_set::{ MapPermission, MemorySet, MapAreaType,VmAreaTree,MapArea,MapType};
use page_table::PTEFlags;
pub use page_table::{
    translated_byte_buffer, get_target_ref, translated_refmut, translated_str, PageTable,get_target_ref_mut,
    PageTableEntry, UserBuffer, UserBufferIterator,fill_str,TranslateRefError
};
use riscv::register::satp;


use crate::sync::Mutex;
 /// The kernel's initial memory mapping(kernel address space)
 pub static  KERNEL_SPACE: LazyInit<Arc<Mutex<MemorySet>>> = LazyInit::new();
 pub static KERNEL_PAGE_TABLE_TOKEN: LazyInit<usize> = LazyInit::new();
 pub static KERNEL_PAGE_TABLE_PPN: LazyInit<PhysPageNum> = LazyInit::new();
/// initiate heap allocator, frame allocator and kernel space
pub fn init() {
   
    heap_allocator::init_heap();
    frame_allocator::init_frame_allocator();
    let ms=MemorySet::new_kernel();
    KERNEL_PAGE_TABLE_TOKEN.init_by(ms.page_table.token());
    
    KERNEL_PAGE_TABLE_PPN.init_by(ms.page_table.root_ppn());
    KERNEL_SPACE.init_by(Arc::new(Mutex::new(ms) ));
    activate_by_token(*KERNEL_PAGE_TABLE_TOKEN);

}
 /// Change page table by writing satp CSR Register use token. 
 pub fn activate_by_token(satp:usize) {
    // trace!("activate new page table token:{:#x}",satp);
    unsafe {
        satp::write(satp);
        asm!("sfence.vma");
    }
    // 
    // trace!("activated");
}
/// the kernel token
pub fn kernel_token() -> usize {
    *KERNEL_PAGE_TABLE_TOKEN
}


pub  fn flush_tlb(){
    unsafe  {
    asm!("sfence.vma");
    }
}