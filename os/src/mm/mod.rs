//! Memory management implementation
//!
//! SV39 page-based virtual-memory architecture for RV64 systems, and
//! everything about memory management, like frame allocator, page table,
//! map area and memory set, is implemented here.
//!
//! Every task or process has a memory_set to control its virtual memory.
pub mod shm;
mod address;
mod area;
pub(crate) mod frame_allocator;
pub mod heap_allocator;
mod memory_set;
pub mod page_table;
use core::{arch::asm, ptr::write_volatile};
pub mod arch;
use alloc::sync::Arc;
use lazy_init::LazyInit;

pub use address::{KernelAddr, PhysAddr, PhysPageNum, StepByOne, VPNRange, VirtAddr, VirtPageNum};
pub use area::{MapArea, MapAreaType, MapPermission, MapType, MmapFile, MmapFlags, VmAreaTree};
pub use frame_allocator::{frame_alloc, frame_dealloc, FrameTracker};
pub use memory_set::MemorySet;
pub use page_table::put_data;

pub use arch::{PTEFlags, PageTableEntry};
pub use page_table::{
    fill_str, get_target_ref, translated_byte_buffer, translated_refmut, translated_str, PageTable,
    TranslateError, UserBuffer, UserBufferIterator,
};

pub const MPOL_DEFAULT: usize = 0;
pub const MPOL_PREFERRED: usize = 1;
pub const MPOL_BIND: usize = 2;
pub const MPOL_INTERLEAVE: usize = 3;
use crate::sync::Mutex;
pub use memory_set::remap_test;
/// The kernel's initial memory mapping(kernel address space)
pub static KERNEL_SPACE: LazyInit<Arc<Mutex<MemorySet>>> = LazyInit::new();
pub static KERNEL_PAGE_TABLE_TOKEN: LazyInit<usize> = LazyInit::new();
pub static KERNEL_PAGE_TABLE_PPN: LazyInit<PhysPageNum> = LazyInit::new();
/// initiate heap allocator, frame allocator and kernel space

pub fn init() {
    heap_allocator::init_heap();
    polyhal::mem::get_mem_areas()
        .cloned()
        .for_each(|(start, size)| {
            info!("memory area: {:#x} - {:#x}", start, start + size);
           
            frame_allocator::add_memory_region(PhysAddr::from(start), PhysAddr::from(start + size));

            
        });
    // frame_allocator::init_frame_allocator();
    let ms = MemorySet::new_kernel();
    
    KERNEL_PAGE_TABLE_TOKEN.init_by(ms.page_table.token());

    KERNEL_PAGE_TABLE_PPN.init_by(ms.page_table.root_ppn());
    KERNEL_SPACE.init_by(Arc::new(Mutex::new(ms)));
    activate_by_token(*KERNEL_PAGE_TABLE_TOKEN);
    
}


/// Change page table by writing satp CSR Register use token.
pub fn activate_by_token(satp: usize) {
    #[cfg(target_arch = "riscv64")]
    unsafe {
        riscv::register::satp::write(satp);
        asm!("sfence.vma");
    }
    #[cfg(target_arch = "loongarch64")]
    // println!("activate satp:{:#x}",satp);
   {
        use loongArch64::register::pgdl;

        use crate::config::PAGE_SIZE_BITS;
        loongArch64::register::pgdl::set_base(satp << PAGE_SIZE_BITS);
        // pgdh::set_base(satp<<PAGE_SIZE_BITS);
   }
        flush_all();
        
}

/// the kernel token
pub fn kernel_token() -> usize {
    *KERNEL_PAGE_TABLE_TOKEN
}

pub fn flush_all() {
    #[cfg(target_arch = "riscv64")]
    unsafe {
        asm!("sfence.vma");
    }
    #[cfg(target_arch = "loongarch64")]
    
    unsafe {
        core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0");
    }
    
}

 #[cfg(target_arch = "loongarch64")]
#[inline]
pub fn  flush_tlb(va:usize){
   
    unsafe {
        core::arch::asm!("dbar 0; invtlb 0x05, $r0, {reg}", reg = in(reg) va);
    }
    

}