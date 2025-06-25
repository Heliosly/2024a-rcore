//! virtio_blk device driver

pub(crate) mod disk;
pub mod manage;
pub(crate) mod virtio_blk;
use core::ptr::NonNull;

use crate::{
    config::{KERNEL_DIRECT_OFFSET, MMIO},
    drivers::block::manage::{BlockDeviceManager, BLOCK_MANAGER},
    fs::{create_file, ext4::ops::Ext4FileSystem, mount, root_inode, OpenFlags, VfsOps, EXT4FS},
};
use alloc::{sync::Arc, vec::Vec};
use disk::Disk;
use lazy_init::LazyInit;
use lazy_static::*;
use spin::mutex::Mutex;
use virtio_blk::VirtioHal;
use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::{
    mmio::{MmioTransport, VirtIOHeader},
    DeviceType, Transport,
};
pub struct SafeMmioTransport(MmioTransport);
unsafe impl Send for SafeMmioTransport {}
unsafe impl Sync for SafeMmioTransport {}
unsafe impl Sync for Disk<VirtioHal, SafeMmioTransport> {}

unsafe impl Send for Disk<VirtioHal, SafeMmioTransport> {}
impl SafeMmioTransport {
    pub fn new() -> Self {
        let header_ptr: usize = MMIO[0].0 + KERNEL_DIRECT_OFFSET;
        let header = NonNull::new(header_ptr as *mut VirtIOHeader).expect("virtio header null");
        SafeMmioTransport(unsafe {
            MmioTransport::new(header).expect("failed to init MMIO transport")
        })
    }
    //  VirtIOBlk::<VirtioHal, MmioTransport>::new(transport)
    //  .expect("failed to create VirtIO blk device")
}

use log::{debug, info};

const VIRTIO_MMIO_START: usize = 0x10001000;
const VIRTIO_MMIO_END: usize = 0x10003000;
const VIRTIO_MMIO_STRIDE: usize = 0x1000;

pub fn find_all_virtio_blk_transports() -> Vec<MmioTransport> {
    let mut transports = Vec::new();
    info!(
        "Scanning for VirtIO devices from {:#x} to {:#x}...",
        VIRTIO_MMIO_START, VIRTIO_MMIO_END
    );

    for addr in (VIRTIO_MMIO_START..VIRTIO_MMIO_END).step_by(VIRTIO_MMIO_STRIDE) {
        let header_ptr = (addr+KERNEL_DIRECT_OFFSET) as *mut VirtIOHeader;

        // 1. 将裸指针转换为 `NonNull`，因为 `MmioTransport::new` 需要它。
        //    `NonNull::new` 会检查指针是否为 null，提供一层安全保障。
        if let Some(non_null_ptr) = NonNull::new(header_ptr) {
            // 2. 尝试创建 `MmioTransport`。这是我们的“探测”步骤。
            //    `MmioTransport::new` 是一个 unsafe 函数，因为它信任我们提供的指针是有效的。
            //    我们在这里调用它是相对安全的，因为我们是在扫描一个已知的 MMIO 区域。
            match unsafe { MmioTransport::new(non_null_ptr) } {
                Ok(transport) => {
                    // 3. 如果创建成功，说明设备有效！现在检查它的类型。
                    debug!("Successfully created transport at {:#x}", addr);
                    if transport.device_type() == DeviceType::Block {
                        info!("Found and initialized a VirtIO BLOCK device at {:#x}", addr);
                        transports.push(transport);
                    } else {
                        info!("Found a VirtIO device at {:#x}, but it's not a block device (Type: {:?}). It will be dropped and reset.", addr, transport.device_type());
                        // `transport` 离开作用域时，它的 Drop 实现会被调用，
                        // 这会自动重置设备 (set_status(0))，使其可被其他驱动程序使用。
                    }
                }
                Err(e) => {
                    // 如果 `MmioTransport::new` 失败（例如 magic number 不对），
                    // 说明这个地址没有一个有效的 VirtIO 设备。
                    debug!("Failed to initialize transport at {:#x}: {:?}", addr, e);
                }
            }
        }
    }

    info!(
        "Scan complete. Found {} block device transport(s).",
        transports.len()
    );
    transports
}

pub fn system_init_with_multi_disk() {
    BLOCK_MANAGER.init_by(spin::Mutex::new(BlockDeviceManager::new()));
    // 1. 扫描并获取所有 VirtIO 块设备的 Transport 实例
    let blk_device_transports = find_all_virtio_blk_transports();

    if blk_device_transports.is_empty() {
        panic!("System startup failed: No VirtIO block devices were found.");
    }

    // 2. 创建设备管理器

    // 3. 遍历已经初始化好的 Transport，并创建上层抽象
    // 我们不再需要处理地址了！
    let mut manage_guard = BLOCK_MANAGER.lock();
    for (i, transport) in blk_device_transports.into_iter().enumerate() {
        info!("Creating VirtIOBlk driver for device #{}", i);

        // a. 使用 Transport 创建 VirtIOBlk 实例
        let virtio_blk = VirtIOBlk::<VirtioHal, _>::new(SafeMmioTransport(transport))
            .expect("Failed to create VirtIOBlk driver instance.");
  
        // b. 将 VirtIOBlk 实例添加到我们的管理器中
        manage_guard.add_disk(virtio_blk);

        info!("Successfully initialized and added block device #{}", i);
    }

    info!("Total disks managed: {}", manage_guard.disk_count());

}
