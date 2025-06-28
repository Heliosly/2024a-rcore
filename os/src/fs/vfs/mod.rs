/// Provides an interface for Virtual File System (VFS) operations.
/// 
/// This module defines traits for filesystem-level and node-level operations,
/// enabling the implementation of various filesystems and their integration
/// into the VFS layer. The `VfsOps` trait represents operations on the
/// filesystem as a whole, while the `VfsNodeOps` trait represents operations
/// on individual files or directories.


pub mod vfs_ops;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use log::{info, warn};

use crate::drivers::{ parse_virtio_device_name, Ext4DiskWrapper};
use crate::fs::ext4::ops::Ext4FileSystem;
// 假设你有一个 Ext4VfsOps 的实现
use crate::fs::mount::{MountEntry, MNT_TABLE};
use crate::fs::{VfsOps, EXT4FS};
use crate::utils::error::{GeneralRet, SysErrNo};

pub struct VfsManager;

impl VfsManager {
    /// 挂载一个新的文件系统。这是 VFS 层的核心 mount 实现。
    pub fn mount(
        special_device: &str,
        mount_point: &str,
        fstype: &str,
        flags: u32,
        _data: Option<String>,
    ) -> GeneralRet {
        info!("VFS: Attempting to mount '{}' ({}) on '{}'", special_device, fstype, mount_point);

        // --- 步骤 1: 驱动层挂载 ---
        // 根据 fstype 创建和初始化文件系统驱动实例
        let fs_instance: Arc<dyn VfsOps> = match fstype {
            "ext4" => {
                // a. 找到并初始化块设备
                let block_id = parse_virtio_device_name(special_device).unwrap();
                println!("init block_id:{}",block_id);
                let disk = Ext4DiskWrapper::new(block_id);
                // b. 创建 Ext4VfsOps 实例
                
                let  mut ext4_fs = Ext4FileSystem::new(disk,special_device.into(),mount_point);
                let res=Arc::new(ext4_fs);
                if special_device=="/dev/vda"{
                   EXT4FS.init_by(res.clone());
                }
                // res.ls();
                res
                
            },
            // "tmpfs" => { /* 创建一个 TmpFs 实例 */ },
            _ => {
                warn!("VFS: Unsupported filesystem type '{}'", fstype);
                return Err(SysErrNo::ENODEV); // No such device (or filesystem)
            }
        };

        // --- 步骤 2: VFS 层挂载 (更新挂载表) ---
        
        // a. 在挂载前，需要确保挂载点目录存在于其父文件系统上
        // let (parent_fs, path_to_dir) = Self::resolve_path_for_parent(mount_point)?;
        // parent_fs.stat(path_to_dir)?; // 检查目录是否存在

        // b. 获取全局挂载表的锁
        let mut mnt_table = MNT_TABLE.lock();
        
        // c. 调用 MountTable 的 mount 方法来记录挂载信息
        // 注意：我们需要一个新版本的 mount 方法来接收 fs_instance
        mnt_table.add_mount_entry(MountEntry {
            special_device: special_device.to_string(),
            mount_point: mount_point.to_string(),
            flags,
            fs_instance,
        })
    }

    /// 卸载一个文件系统
    pub fn umount(path_or_device: &str) -> GeneralRet {
        // ... umount 逻辑，需要先从 MountTable 获取 fs_instance，
        // 调用 fs_instance 的 umount 方法（如果需要），然后再从表中移除 ...
        MNT_TABLE.lock().umount(path_or_device).map(|_|())
    }

    /// 解析一个绝对路径，找到应该处理它的文件系统和其在该文件系统内的相对路径
    pub fn resolve_mnt_path(abs_path: &str) -> (Arc<dyn VfsOps>, String) {
        let mnt_table = MNT_TABLE.lock();

        // 查找最长匹配的挂载点
        // 一个简单的实现是遍历所有条目，找到最长的前缀匹配
        let mut best_match: Option<(&MountEntry, usize)> = None;

        for entry in &mnt_table.entries {
            if abs_path.starts_with(&entry.mount_point) {
                let mount_len = entry.mount_point.len();
                if best_match.is_none() || mount_len > best_match.unwrap().1 {
                    best_match = Some((entry, mount_len));
                }
            }
        }

        if let Some((entry, mount_len)) = best_match {
            // 计算相对路径
            let mut path_in_fs = &abs_path[mount_len..];
            let path_in_fs = if !path_in_fs.starts_with('/') {
                format!("/{}", path_in_fs)
            } else {
                path_in_fs.to_string()
            };
            
            return (entry.fs_instance.clone(), path_in_fs.to_string());
        }

        // 如果没有找到任何挂载点（这不应该发生，因为至少有根'/'）
        println!("SysFile Not any Mountpoint");
        unreachable!()
    }

    
}