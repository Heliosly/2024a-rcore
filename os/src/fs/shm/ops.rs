use crate::{fs::{vfs::vfs_ops::VfsNodeOps, VfsOps}, mm::shm::ShmManager, sync::Mutex, utils::error::GeneralRet};
// 在 fs/shmfs.rs 中
use alloc::{collections::BTreeMap, string::String, sync::Arc};

pub struct ShmFs {
    /// 文件系统内的 "文件" 列表：文件名 -> Inode
    files: Mutex<BTreeMap<String, Arc<ShmFsInode>>>,
    /// 指向全局的共享内存管理器
    shm_manager: Arc<Mutex<ShmManager>>,
}

impl ShmFs {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(BTreeMap::new()),
            // 从全局静态变量获取管理器
            shm_manager: SHM_MANAGER.get().unwrap().clone(),
        }
    }
}

impl VfsOps for ShmFs {
    /// 文件系统的根 Inode
    fn root_inode(&self) -> Arc<dyn VfsNodeOps> {

    }
    fn sync(&self)->GeneralRet{
        return Ok(())
    }

    /// 在文件系统中查找一个名为 `name` 的 Inode
    async fn find(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let files: crate::sync::MutexGuard<'_, BTreeMap<String, Arc<_>>> = self.files.lock().await;
        files.get(name).map(|inode| inode.clone() as Arc<dyn Inode>)
    }

    /// 创建一个名为 `name` 的文件，这对应于 shm_open 的创建逻辑
    async fn create(&self, name: &str, size: usize) -> Result<Arc<dyn Inode>, SysErrNo> {
        let mut files = self.files.lock().await;
        if files.contains_key(name) {
            return Err(SysErrNo::EEXIST);
        }

        // 1. 在 SHM_MANAGER 中创建一个新的共享内存段
        let mut manager = self.shm_manager.lock().await;
        let key = // 我们可以用哈希(name)或一个原子计数器作为内部 key
        // let shmid = manager.create_segment_with_key_and_size(key, size)?; // 假设有这个函数

        // 2. 创建一个 ShmFsInode 来代表这个段
        // let inode = Arc::new(ShmFsInode::new(name, shmid, self.shm_manager.clone()));
        
        // 3. 将它添加到文件系统目录中
        files.insert(String::from(name), inode.clone());

        Ok(inode as Arc<dyn Inode>)
    }
}
