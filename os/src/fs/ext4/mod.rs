use alloc::sync::Arc;
use ops::Ext4FileSystem;
use spin::Lazy;
use virtio_drivers::device::blk::VirtIOBlk;

use crate::drivers::block::{disk::Disk, SafeMmioTransport};

use crate::drivers::block::virtio_blk::VirtioHal;
use super::VfsOps;

pub mod ops;




pub static EXT4FS: Lazy<Arc<dyn VfsOps>> = Lazy::new(|| {
   Arc::new(Ext4FileSystem::new(Disk::<VirtioHal,SafeMmioTransport>::new(VirtIOBlk::new(SafeMmioTransport::new()).expect("failed to create VirtIO blk device"))))
});