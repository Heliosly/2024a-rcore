use alloc::{
    string::String , 
    sync::Arc,
    vec::Vec,
};
use core::fmt; 
use spin::{Lazy, Mutex};
use log:: info ;

use crate::{config::MNT_TABLE_MAX_ENTRIES, fs::VfsOps, utils::error::{GeneralRet, SysErrNo}};


const MS_RDONLY: u32 = 1;      // Mount read-only
const MS_NOSUID: u32 = 2;      // Ignore SUID and SGID bits
const MS_NODEV: u32 = 4;       // Disallow access to device special files
const MS_NOEXEC: u32 = 8;      // Disallow program execution
const MS_SYNCHRONOUS: u32 = 16; // Writes are synced at once
const MS_REMOUNT: u32 = 32;    // Remount a mounted filesystem
const MS_MANDLOCK: u32 = 64;   // Allow mandatory locks on an FS

#[derive(Clone)]
pub struct MountEntry {
    pub special_device: String,
    pub mount_point: String,
    // pub filesystem_type: String, // 这个信息可以从 fs_instance 中获取
    pub flags: u32,
    // 核心改动：存储一个指向活动文件系统实例的引用
    pub fs_instance: Arc<dyn VfsOps>, 
}

// ... MountEntry 的 Debug impl 也需要相应更新 ...
impl fmt::Debug for MountEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountEntry")
            .field("special", &self.special_device)
            .field("dir", &self.mount_point)
            .field("fstype", &self.fs_instance.name())
            .field("flags", &self.flags)
            .finish()
    }
}

/// 内核的挂载表
pub struct MountTable {
    pub entries: Vec<MountEntry>, // 存储所有挂载点信息
}

impl MountTable {

    pub fn add_mount_entry(&mut self, new_entry: MountEntry) -> GeneralRet {
        // 检查逻辑 (如 EBUSY) 保持不变
        if self.entries.iter().any(|e| e.mount_point == new_entry.mount_point) {
            return Err(SysErrNo::EBUSY);
        }
        
        self.entries.push(new_entry);
        Ok(())
    }
    /// 创建一个新的空挂载表。
    pub fn new() -> Self {
        MountTable {
            entries: Vec::new(),
        }
    }
  /// 在挂载表中注册一个新的挂载条目。
    /// 这个函数现在是 VFS 层的内部函数，由 VfsManager 调用。
    /// 它的职责是簿记和规则检查。
    pub fn mount(&mut self, new_entry: MountEntry) -> GeneralRet {
        debug!("MountTable::mount: registering entry {:?}", new_entry);

        // 检查挂载表是否已满
        if self.entries.len() >= MNT_TABLE_MAX_ENTRIES {
            warn!("Mount table full (max {} entries)", MNT_TABLE_MAX_ENTRIES);
            return Err(SysErrNo::ENOMEM);
        }

        // 检查挂载点 'dir' 是否已存在
        if self.entries.iter().any(|entry| entry.mount_point == new_entry.mount_point) {
            // 注意：重新挂载 (remount) 的逻辑应该在 VfsManager 中处理，
            // 因为它可能需要与文件系统驱动交互来改变标志。
            // MountTable 只负责简单的检查。
            warn!("Mount point '{}' already in use (EBUSY)", new_entry.mount_point);
            return Err(SysErrNo::EBUSY);
        }

        // 检查特殊设备 'special' 是否已被挂载
        if self.entries.iter().any(|entry| entry.special_device == new_entry.special_device) {
            // 允许一些虚拟文件系统（如 proc, tmpfs）的特殊设备名为 "none" 并重复
            if !new_entry.special_device.is_empty() && new_entry.special_device != "none" {
                 warn!("Special device '{}' already mounted (EBUSY)", new_entry.special_device);
                 return Err(SysErrNo::EBUSY);
            }
        }

        info!("Registering mount: '{}' on '{}' as type '{}'", 
              new_entry.special_device, new_entry.mount_point, new_entry.fs_instance.name());

        self.entries.push(new_entry);
        
        // 按挂载点路径长度降序排序，方便 `resolve_path` 进行最长匹配
        self.entries.sort_by(|a, b| b.mount_point.len().cmp(&a.mount_point.len()));

        Ok(())
    }
    
    // umount, get_mount_info_by_dir 等其他函数也需要相应调整...
    pub fn umount(&mut self, path_or_device: &str) -> Result<Arc<dyn VfsOps>, SysErrNo> {
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.mount_point == path_or_device || entry.special_device == path_or_device
        }) {
            // 移除条目并返回被移除的文件系统实例，以便 VFS 可以调用它的卸载逻辑
            let removed_entry = self.entries.remove(index);
            info!("Unregistered mount for '{}'", path_or_device);
            Ok(removed_entry.fs_instance)
        } else {
            warn!("Target '{}' not found in mount table for unmount", path_or_device);
            Err(SysErrNo::EINVAL)
        }
    }
/// 根据目录路径查找挂载信息。
    /// 如果给定的 `dir` 是一个挂载点，则返回其挂载信息。
    pub fn get_mount_info_by_dir(&self, dir_path: &str) -> Option<MountEntry> {
        self.entries
            .iter()
            .find(|entry| entry.mount_point == dir_path)
            .cloned() // 返回 MountEntry 的克隆
    }
   
}

/// 全局挂载表实例。
/// 使用 `Lazy` 来延迟初始化，`Arc<Mutex<...>>` 来实现线程安全的共享访问。
pub static MNT_TABLE: Lazy<Arc<Mutex<MountTable>>> = Lazy::new(|| {
    info!("Initializing global mount table.");
    Arc::new(Mutex::new(MountTable::new()))
});


