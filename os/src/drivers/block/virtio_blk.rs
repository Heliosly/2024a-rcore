use core::ptr::NonNull;

use crate::{
     drivers::block::DMA_PADDR, mm::{frame_dealloc, kernel_token, KernelAddr, PageTable, PhysPageNum, StepByOne, VirtAddr}
};

use virtio_drivers::{
    transport:: Transport,
    BufferDirection, Hal, PhysAddr, PAGE_SIZE,
};
use super::SafeMmioTransport as MmioTransport;
use log::trace;



pub struct VirtioHal;

unsafe impl Hal for VirtioHal {
    /// 分配 `pages` 页 DMA 内存，返回 (物理地址, 内核可访问的虚拟指针)
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        // 从全局 DMA_PADDR 取出一段连续的物理区
        let paddr_usize =
            DMA_PADDR.fetch_add(pages * PAGE_SIZE, core::sync::atomic::Ordering::SeqCst);
        let paddr = PhysAddr::from(paddr_usize);

        // 计算对应的内核虚地址
        let vaddr_usize = KernelAddr::from(paddr).0;
        let vaddr = NonNull::new(vaddr_usize as *mut u8).expect("virtio_hal: dma_alloc vaddr null");

        trace!(
            "dma_alloc: phys={:#x}, virt={:#x}, pages={}",
            paddr_usize,
            vaddr_usize,
            pages
        );
        (paddr, vaddr)
    }

    /// 释放之前分配的 DMA 内存
    unsafe fn dma_dealloc(paddr: PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        let mut ppn: PhysPageNum = paddr.into();
        for _ in 0..pages {
            frame_dealloc(ppn);
            ppn.step();
        }
        trace!("dma_dealloc: phys={:#x}, pages={}", paddr, pages);
        0
    }

    /// 将 MMIO 物理地址映射到内核虚地址
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        let vaddr_usize = KernelAddr::from(paddr).0;
        NonNull::new(vaddr_usize as *mut u8).expect("virtio_hal: mmio_phys_to_virt null")
    }

    /// 在某些平台下，可能需要把 buffer 标记为可被设备访问
    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        let vptr = buffer.as_ptr() as *const u8 as usize;

        let paddr_usize = PageTable::from_token(kernel_token())
            .translate_va(VirtAddr::from(vptr))
            .expect("virtio_hal: share translate failed")
            .0;
        PhysAddr::from(paddr_usize)
    }

    /// 取消 share（如无缓存，可以空实现）
    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // nothing to do
    }
}

impl Transport for MmioTransport{
    fn device_type(&self) -> virtio_drivers::transport::DeviceType {
        self.0.device_type()
    }

    fn read_device_features(&mut self) -> u64 {
        self.0.read_device_features()
    }

    fn write_driver_features(&mut self, driver_features: u64) {
        self.0.write_driver_features(driver_features)
    }

    fn max_queue_size(&mut self, queue: u16) -> u32 {
        self.0.max_queue_size(queue)
    }

    fn notify(&mut self, queue: u16) {
        self.0.notify(queue)
    }

    fn get_status(&self) -> virtio_drivers::transport::DeviceStatus {
        self.0.get_status()
    }

    fn set_status(&mut self, status: virtio_drivers::transport::DeviceStatus) {
        self.0.set_status(status)
    }

    fn set_guest_page_size(&mut self, guest_page_size: u32) {
        self.0.set_guest_page_size(guest_page_size)
    }

    fn requires_legacy_layout(&self) -> bool {
        self.0.requires_legacy_layout()
    }

    fn queue_set(
        &mut self,
        queue: u16,
        size: u32,
        descriptors: PhysAddr,
        driver_area: PhysAddr,
        device_area: PhysAddr,
    ) {
        self.0.queue_set(queue, size, descriptors, driver_area, device_area)
    }

    fn queue_unset(&mut self, queue: u16) {
        self.0.queue_unset(queue)
    }

    fn queue_used(&mut self, queue: u16) -> bool {
        self.0.queue_used(queue)
    }

    fn ack_interrupt(&mut self) -> bool {
        self.0.ack_interrupt()
    }

    fn config_space<T: 'static>(&self) -> virtio_drivers::Result<NonNull<T>> {
        self.0.config_space()
    }
}