use lazy_init::LazyInit;
use virtio_drivers::{
    device::blk::VirtIOBlk,
    transport::Transport, Hal,
};
use alloc::{sync::Arc, vec::Vec};

use spin::Mutex;
use crate::drivers::block::{disk::Disk, virtio_blk::VirtioHal, SafeMmioTransport}; 
pub type GlobalBdm = BlockDeviceManager<CHal,CTransport>;
pub type CHal=VirtioHal;
pub type CTransport = SafeMmioTransport;
/// Manages a collection of block devices.
///
/// This struct holds `Disk` instances, which are wrappers around the
/// low-level `VirtIOBlk` drivers. It allows the system to manage multiple
/// disks and retrieve them by an ID.
///
/// # Type Parameters
/// * `H`: The HAL implementation, must implement `virtio_drivers::Hal`.
/// * `T`: The transport implementation, must implement `virtio_drivers::transport::Transport`.

/// 管理块设备，不再滑动索引，通过 Vec<Option<>> 保留空位
pub struct BlockDeviceManager<H: Hal, T: Transport> {
    disks: Vec<Option<Disk<H, T>>>,
}

impl<H: Hal, T: Transport> Default for BlockDeviceManager<H, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Hal, T: Transport> BlockDeviceManager<H, T> {
    /// 创建一个新的、空的管理器
    pub fn new() -> Self {
        Self { disks: Vec::new() }
    }

    /// 添加一个新的 `VirtIOBlk` 设备，返回它的 ID
    pub fn add_disk(&mut self, device: VirtIOBlk<H, T>) -> usize {
        let disk = Disk::new(device);
        // 尝试复用空位
        if let Some(pos) = self.disks.iter().position(|slot| slot.is_none()) {
            self.disks[pos] = Some(disk);
            pos
        } else {
            self.disks.push(Some(disk));
            self.disks.len() - 1
        }
    }

    /// 获取可变引用
    pub fn get_disk_mut(&mut self, id: usize) -> Option<&mut Disk<H, T>> {
        self.disks.get_mut(id)?.as_mut()
    }

    /// 取出一个设备，但不改变其他索引
    pub fn take_disk(&mut self, id: usize) -> Option<Disk<H, T>> {
        if id < self.disks.len() {
            self.disks[id].take()
        } else {
            None
        }
    }

    /// 获取共享引用
    pub fn get_disk(&self, id: usize) -> Option<&Disk<H, T>> {
        self.disks.get(id)?.as_ref()
    }

    /// 返回当前管理的设备总数（包含空位）
    pub fn capacity(&self) -> usize {
        self.disks.len()
    }

    /// 返回实际在用的设备数量（不含空位）
    pub fn disk_count(&self) -> usize {
        self.disks.iter().filter(|d| d.is_some()).count()
    }
}

/// 全局单例示例





pub static BLOCK_MANAGER:  LazyInit<Mutex<GlobalBdm>> = LazyInit::new();