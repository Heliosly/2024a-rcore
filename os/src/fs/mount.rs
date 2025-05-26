use alloc::{
    string::{String, ToString}, 
    sync::Arc,
    vec::Vec,
};
use core::fmt; 
use spin::{Lazy, Mutex};
use log::{debug, info };


const MS_RDONLY: u32 = 1;      // Mount read-only
const MS_NOSUID: u32 = 2;      // Ignore SUID and SGID bits
const MS_NODEV: u32 = 4;       // Disallow access to device special files
const MS_NOEXEC: u32 = 8;      // Disallow program execution
const MS_SYNCHRONOUS: u32 = 16; // Writes are synced at once
const MS_REMOUNT: u32 = 32;    // Remount a mounted filesystem
const MS_MANDLOCK: u32 = 64;   // Allow mandatory locks on an FS

/// 表示一个挂载点条目
#[derive(Clone)]
pub struct MountEntry {
    pub special_device: String, // 特殊设备路径 (例如 /dev/sda1)
    pub mount_point: String,    // 挂载目录路径 (例如 /mnt)
    pub filesystem_type: String,// 文件系统类型 (例如 ext4, fat32)
    pub flags: u32,             // 挂载标志 (例如 MS_RDONLY)
    // data 字段通常是文件系统特定的挂载选项字符串，这里我们暂时不详细处理
    // pub data: Option<String>,
}

// 为 MountEntry 实现 Debug trait 以便打印
impl fmt::Debug for MountEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountEntry")
            .field("special", &self.special_device)
            .field("dir", &self.mount_point)
            .field("fstype", &self.filesystem_type)
            .field("flags", &self.flags)
            .finish()
    }
}


/// 内核的挂载表
pub struct MountTable {
    entries: Vec<MountEntry>, // 存储所有挂载点信息
}

impl MountTable {
    /// 创建一个新的空挂载表。
    pub fn new() -> Self {
        MountTable {
            entries: Vec::new(),
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


