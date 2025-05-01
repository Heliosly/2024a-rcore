//! virtio_blk device driver

mod virtio_blk;
pub(crate) mod disk;
use core::{ ptr::NonNull, sync::atomic::{AtomicUsize, Ordering}};

use disk::Disk;
use lazy_static::*;
use virtio_blk:: VirtioHal;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use spin::mutex::Mutex;
use virtio_drivers::device::blk::VirtIOBlk;
use crate::config;
pub struct SafeMmioTransport(MmioTransport);
unsafe impl Send for SafeMmioTransport {}
unsafe impl Sync for SafeMmioTransport {}
unsafe impl Sync for Disk<VirtioHal,  SafeMmioTransport>{}

unsafe impl Send for Disk<VirtioHal,  SafeMmioTransport>{}
const DMA_PADDR: AtomicUsize = AtomicUsize::new(0x1000_1000 + config::KERNEL_DIRECT_OFFSET);
impl SafeMmioTransport{

    pub fn new()->Self{
         let header_ptr: usize = DMA_PADDR.load(Ordering::SeqCst);
     let header = NonNull::new(header_ptr as *mut VirtIOHeader).expect("virtio header null");
   SafeMmioTransport(  unsafe { MmioTransport::new(header).expect("failed to init MMIO transport") } 
     )
    }
    //  VirtIOBlk::<VirtioHal, MmioTransport>::new(transport)
    //  .expect("failed to create VirtIO blk device")
 }




lazy_static! {    /// The global block device driver  Safence: BLOCK_DEVICE with BlockDevice trait
    pub static ref BLOCK_DEVICE: Mutex<Disk<VirtioHal,  SafeMmioTransport>> =Mutex::new(Disk::new(
        VirtIOBlk::new(SafeMmioTransport::new())
            .expect("failed to create VirtIO blk device")
    ));
}



// #[allow(unused)]
// /// Test the block device
// pub fn block_device_test() {
//     let block_device = BLOCK_DEVICE.clone();
//     let mut write_buffer = [0u8; 512];
//     let mut read_buffer = [0u8; 512];
//     for i in 0..512 {
//         for byte in write_buffer.iter_mut() {
//             *byte = i as u8;
//         }
//         block_device.write_block(i as usize, &write_buffer);
//         block_device.read_block(i as usize, &mut read_buffer);
//         assert_eq!(write_buffer, read_buffer);
//     }
//     println!("block device test passed!");
// }
