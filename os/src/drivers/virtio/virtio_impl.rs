use alloc::{sync::Arc, vec::Vec};
use polyhal::VirtAddr;
use core::ptr::NonNull;
use devices::{  VIRT_ADDR_START};
use log::trace;
use virtio_drivers::{BufferDirection, Hal, PhysAddr as DMAPhysAddr};

use crate::{config::{KERNEL_DIRECT_OFFSET, PAGE_SIZE_BITS}, mm::{frame_alloc, frame_allocator::frame_alloc_continue, FrameTracker, KernelAddr, PhysAddr, PhysPageNum}, sync::UPSafeCell};

use lazy_static::lazy_static;

lazy_static! {
    static ref QUEUE_FRAMES: UPSafeCell<Vec<Arc<FrameTracker>>> = unsafe { UPSafeCell::new(Vec::new()) };
}

pub struct HalImpl;

 unsafe impl Hal for HalImpl {
    /// 分配 `pages` 页 DMA 内存，返回 (物理地址, 内核可访问的虚拟指针)
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (DMAPhysAddr, NonNull<u8>) {
         // 1) 分配真正的、连续的物理页
         info!("dma_alloc");
         let trackers = frame_alloc_continue(pages).expect("can't alloc page in virtio");
         let pa  =PhysAddr::from( trackers[0].ppn);
         let paddr = trackers[0].ppn.0<<PAGE_SIZE_BITS;

         let vaddr = NonNull::new(pa.get_mut_ptr() ).unwrap();
         trace!("alloc DMA: paddr={:#x}, pages={:?}", paddr, trackers);
         QUEUE_FRAMES.exclusive_access().extend(trackers);
         (paddr, vaddr)
    }

    /// 释放之前分配的 DMA 内存
    unsafe fn dma_dealloc(paddr: DMAPhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        trace!("dealloc DMA: paddr={:#x}, pages={}", paddr, pages);
        // VIRTIO_CONTAINER.lock().drain_filter(|x| {
        //     let phy_page = paddr as usize >> 12;
        //     let calc_page = usize::from(x.0);

        //     calc_page >= phy_page && calc_page - phy_page < pages
        // });
        QUEUE_FRAMES.exclusive_access().retain(|x| {
            let phy_page = paddr >> 12;
            let calc_page = x.ppn();

            !(phy_page..phy_page + pages).contains(&calc_page.0)
        });
        0
    }

    /// 将 MMIO 物理地址映射到内核虚地址
    unsafe fn mmio_phys_to_virt(paddr: usize, _size: usize) -> NonNull<u8> {
        NonNull::new((paddr | VIRT_ADDR_START) as *mut u8).unwrap()
    }

    /// 在某些平台下，可能需要把 buffer 标记为可被设备访问
    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> usize {
        let raw_ptr = buffer.as_ptr() as *mut u8 as usize;
        // Nothing to do, as the host already has access to all memory.
        devices::utils::virt_to_phys(raw_ptr).unwrap_or(raw_ptr & (!VIRT_ADDR_START))
        // buffer.as_ptr() as *mut u8 as usize - VIRT_ADDR_START
        
    }

    /// 取消 share（如无缓存，可以空实现）
    unsafe fn unshare(_paddr: DMAPhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // nothing to do
    }
}
