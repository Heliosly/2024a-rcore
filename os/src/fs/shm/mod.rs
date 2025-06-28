// mod ops;
use crate::fs::stat::StMode;
use crate::fs::vfs::vfs_ops::VfsNodeOps;
use crate::mm::shm::{SharedMemorySegment, ShmManager};
use crate::fs::{File, Kstat, OpenFlags }; use alloc::string::String;
// 假设您有 Inode trait
use alloc::sync::Arc;
use crate::sync::Mutex;

/// 代表 /dev/shm/ 下的一个 "文件" 的 Inode。
/// 它本质上是 ShmManager 中一个 SharedMemorySegment 的包装。
pub struct ShmFsInode {
    /// 文件名，例如 "cyclist6"
    name: String,
    /// 对应的共享内存段的 ID
    shmid: i32,
    /// 指向全局 SHM 管理器
    manager: Arc<Mutex<ShmManager>>, 
    flag:OpenFlags
}

impl ShmFsInode {
    pub fn new(name: &str, shmid: i32, manager: Arc<Mutex<ShmManager>>,flag:OpenFlags) -> Self {
        Self {
            name: String::from(name),
            shmid,
            manager,
            flag
        }
    }
}

// ShmFsInode 需要实现 File trait，以便 open() 后返回一个可操作的对象
impl VfsNodeOps for ShmFsInode {
    // readable/writable/read/write/lseek 等方法可以返回错误或无操作，
    // 因为对它的主要操作是 mmap。
    fn fstat(&self) -> Kstat {
        // 元数据可以从 SharedMemorySegment 中获取
        let manager = self.manager.lock(); // 这里使用同步锁，因为 fstat 不应是 async
        let segment_arc = manager.id_to_segment.get(&self.shmid).unwrap();
        let segment = segment_arc.lock();

        Kstat {
            st_mode: StMode::FREG.bits() | 0o666, // 表现为常规文件
            st_nlink: segment.id_ds.shm_nattch + 1, // 附加数 + 目录引用
            st_size: segment.id_ds.shm_segsz as u64,
            // ... 其他字段 ...
            ..Kstat::default()
        }
    }    /// 在文件系统中查找一个名为 `name` 的 Inode
    async fn find(&self,
        path: &str,
        flags: OpenFlags,
        loop_times: usize) -> Option<Arc<dyn VfsNodeOps>> {
        let files: crate::sync::MutexGuard<'_, BTreeMap<String, Arc<_>>> = self.files.lock().await;
        files.get(name).map(|inode| inode.clone() as Arc<dyn Inode>)
    }

}