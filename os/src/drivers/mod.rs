//! block device driver


pub mod block;
pub use block::system_init_with_multi_disk;

use crate::{drivers::block::{disk::Disk, manage::{CHal, CTransport, BLOCK_MANAGER}}, utils::error::{SysErrNo, TemplateRet}};

/// 根据设备名（如 "/dev/vda"）查找一个块设备。
pub fn find_block_device(name: &str) -> TemplateRet<Disk<CHal,CTransport >> {
    // 1. 解析设备名，获取索引
    let index = match parse_virtio_device_name(name) {
        Some(i) => i,
        None => {
            warn!("Failed to parse block device name: '{}'", name);
            return Err(SysErrNo::ENODEV); // No such device
        }
    };

    // 2. 从全局 BLOCK_MANAGER 中获取设备
    let mut manager = BLOCK_MANAGER.lock();
    
    // 注意：get_disk_mut 返回的是 &mut Disk。但通常我们希望 VFS 持有的是
    // 一个可以共享的、内部可变的引用，比如 Arc<Mutex<Disk>>。
    // 这需要在 BlockDeviceManager 中做一些调整。我们先假设可以直接获取。
    // 我们将在下面讨论如何改进 BlockDeviceManager。
    
    // 暂时假设我们能获取一个可用的 Disk 引用并返回
    // 实际实现会更复杂，见下面的讨论。
    if let Some(disk_ref) = manager.take_disk(index) { // 假设有这个方法
        Ok(disk_ref)
    } else {
        warn!("Block device with index {} not found for name '{}'", index, name);
 
        Err(SysErrNo::ENODEV)
    }
}

/// 解析 virtio 设备名，如 "/dev/vda", "/dev/vdb" 等，返回其索引。
/// "/dev/vda" -> 0, "/dev/vdb" -> 1, ...
fn parse_virtio_device_name(name: &str) -> Option<usize> {
    const PREFIX: &str = "/dev/vd";
    if !name.starts_with(PREFIX) {
        return None;
    }

    // 获取前缀后面的字符
    let suffix = &name[PREFIX.len()..];
    if suffix.len() == 1 {
        let last_char = suffix.chars().next()?;
        if last_char.is_ascii_lowercase() {
            // 计算索引 'a' -> 0, 'b' -> 1, ...
            Some((last_char as u32 - 'a' as u32) as usize)
        } else {
            None
        }
    } else {
        None
    }
}