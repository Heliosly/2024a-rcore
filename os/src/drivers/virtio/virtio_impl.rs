use alloc::{sync::Arc, vec::Vec};
use polyhal::VirtAddr;
use spin::Mutex;
use core::ptr::NonNull;
use crate::devices::{  utils::virt_to_phys, VIRT_ADDR_START};
use log::trace;
use virtio_drivers::{BufferDirection, Hal, PhysAddr as DMAPhysAddr};

use crate::{config::{KERNEL_DIRECT_OFFSET, PAGE_SIZE_BITS}, mm::{frame_alloc, frame_allocator::frame_alloc_continue, FrameTracker, KernelAddr, PhysAddr, PhysPageNum}, sync::UPSafeCell};

use lazy_static::lazy_static;
static VIRTIO_CONTAINER: Mutex<Vec<Arc<FrameTracker>>> = Mutex::new(Vec::new());

pub struct HalImpl;

unsafe impl Hal for HalImpl {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (usize, NonNull<u8>) {
        let trackers = frame_alloc_continue(pages).expect("can't alloc page in virtio");
        let paddr = PhysAddr::from(trackers[0].ppn);
        let vaddr = NonNull::new(paddr.get_mut_ptr() ).unwrap();
        trace!("alloc DMA: paddr={:#x}, pages={:?}", paddr.0, trackers);
        VIRTIO_CONTAINER.lock().extend(trackers);
        (paddr.0, vaddr)
    }

    unsafe fn dma_dealloc(paddr: usize, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        trace!("dealloc DMA: paddr={:#x}, pages={}", paddr, pages);
        // VIRTIO_CONTAINER.lock().drain_filter(|x| {
        //     let phy_page = paddr as usize >> 12;
        //     let calc_page = usize::from(x.0);

        //     calc_page >= phy_page && calc_page - phy_page < pages
        // });
        VIRTIO_CONTAINER.lock().retain(|x| {
            let phy_page = paddr >> 12;
            let calc_page = x.ppn().raw();

            !(phy_page..phy_page + pages).contains(&calc_page)
        });
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: usize, _size: usize) -> NonNull<u8> {
        NonNull::new((paddr | VIRT_ADDR_START) as *mut u8).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> usize {
        let raw_ptr = buffer.as_ptr() as *mut u8 as usize;
        // Nothing to do, as the host already has access to all memory.
        virt_to_phys(raw_ptr).unwrap_or(raw_ptr & (!VIRT_ADDR_START))
        // buffer.as_ptr() as *mut u8 as usize - VIRT_ADDR_START
    }

    unsafe fn unshare(_paddr: usize, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // Nothing to do, as the host already has access to all memory and we didn't copy the buffer
        // anywhere else.
    }
}
