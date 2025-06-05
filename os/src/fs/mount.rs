use alloc::{
    string::String , 
    sync::Arc,
    vec::Vec,
};
use core::fmt; 
use spin::{Lazy, Mutex};
use log:: info ;

use crate::{config::MNT_TABLE_MAX_ENTRIES, utils::error::{GeneralRet, SysErrNo}};


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
    /// 挂载一个新的文件系统。
    ///
    /// # 参数
    /// * `special`: 要挂载的特殊设备（例如块设备路径）。
    /// * `dir`: 文件系统要挂载到的目录路径。
    /// * `fstype`: 文件系统的类型字符串。
    /// * `flags`: 挂载标志的位掩码。
    /// * `data`: 文件系统特定的挂载选项字符串（当前版本可能未使用）。
    ///
    /// # 返回
    /// `Ok(())` 如果挂载成功。
    /// `Err(SysErrNo)` 如果发生错误，例如挂载点已存在（除非是 remount）、表已满等。
    pub fn mount(
        &mut self,
        special: String,
        dir: String,
        fstype: String,
        flags: u32,
        data: Option<String>, // data 参数通常是可选的
) -> GeneralRet{
        debug!(
            "MountTable::mount: special='{}', dir='{}', fstype='{}', flags=0x{:x}",
            special, dir, fstype, flags
        );

        // 检查挂载表是否已满
        if self.entries.len() >= MNT_TABLE_MAX_ENTRIES {
            warn!("Mount table full (max {} entries)", MNT_TABLE_MAX_ENTRIES);
            return Err(SysErrNo::ENOMEM); // 或者一个更合适的错误码，如 EBUSY 或 ENFILE
        }

        // 检查挂载点 'dir' 是否已存在
        if let Some(existing_entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.mount_point == dir)
        {
            // 挂载点已存在
            if flags & MS_REMOUNT != 0 {
                // 如果包含 MS_REMOUNT 标志，则执行重新挂载操作
                info!("Remounting '{}' to '{}' with fstype '{}'", dir, special, fstype);
                existing_entry.special_device = special;
                existing_entry.filesystem_type = fstype;
                existing_entry.flags = flags; // 更新标志
                // data 字段也可能需要更新
                return Ok(());
            } else {
                // 挂载点已存在，且不是重新挂载 -> 错误
                warn!("Mount point '{}' already in use (EBUSY)", dir);
                return Err(SysErrNo::EBUSY); // Device or resource busy
            }
        }

        // 检查特殊设备 'special' 是否已被挂载 (通常一个设备只能挂载一次，除非特殊情况)
        if self.entries.iter().any(|entry| entry.special_device == special) {
            if fstype != "proc" && fstype != "tmpfs" { // 示例：允许 proc 和 tmpfs 重复 "none"
                 warn!("Special device '{}' already mounted (EBUSY)", special);
                 return Err(SysErrNo::EBUSY);
            }
        }


        // TODO: 实际的文件系统挂载逻辑
        // 1. 查找或加载 fstype 对应的文件系统驱动。
        // 2. 打开 special 设备。
        // 3. 调用文件系统驱动的 mount 方法，传入设备、挂载点inode、flags、data。
        // 4. 如果成功，文件系统驱动会返回一个代表已挂载文件系统的根inode或超级块。
        // 5. 将挂载点目录的 inode 标记为挂载点，并将其与新挂载的文件系统的根关联。
        //    (例如，修改挂载点目录 inode 的 lookup 方法，使其指向新文件系统的根)
        // 目前，我们只将信息添加到挂载表中。
        info!("Mounting '{}' on '{}' as type '{}' with flags 0x{:x}", special, dir, fstype, flags);

        self.entries.push(MountEntry {
            special_device: special,
            mount_point: dir,
            filesystem_type: fstype,
            flags,
            // data: data,
        });
        Ok(())
    }
    pub fn umount(&mut self, path_or_device: &str, flags: u32) -> Result<(), SysErrNo> {
        debug!("MountTable::umount: target='{}', flags=0x{:x}", path_or_device, flags);
        // TODO: 实现 MNT_FORCE (强制卸载，即使繁忙) 和 MNT_DETACH (懒卸载) 的逻辑
        // 这通常需要与文件系统和 VFS 交互，检查是否有打开的文件或繁忙的资源。

        let original_len = self.entries.len();
        self.entries.retain(|entry| {
            // 如果 entry 的挂载点或设备与 path_or_device 匹配，则不保留 (即移除)
            !(entry.mount_point == path_or_device || entry.special_device == path_or_device)
        });

        if self.entries.len() < original_len {
            // 成功移除了至少一个条目
            // TODO: 实际的文件系统卸载逻辑
            // 1. 通知文件系统驱动执行卸载操作（例如同步数据、释放资源）。
            // 2. 清理挂载点 inode 的状态，使其不再指向已卸载文件系统的根。
            info!("Successfully unmounted '{}'", path_or_device);
            Ok(())
        } else {
            warn!("Target '{}' not found in mount table for unmount (EINVAL/ENOENT)", path_or_device);
            Err(SysErrNo::EINVAL) // 或者 ENOENT，取决于哪个更合适
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


