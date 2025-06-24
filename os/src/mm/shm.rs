use core::sync::atomic::AtomicUsize;

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};
use lazy_init::LazyInit;

use crate::{
    config::PAGE_SIZE,
    mm::{frame_allocator::frame_alloc_continuous, FrameTracker, PhysPageNum},
    sync::Mutex,
};

pub struct SharedMemoryBacking {
    shard: Mutex<SharedMemorySegment>,
}

// for shmget
pub const IPC_CREAT: usize = 0o1000;
pub const IPC_EXCL: usize = 0o2000;
pub const IPC_PRIVATE: i32 = 0;

// for shmctl
pub const IPC_RMID: usize = 0;
pub const IPC_SET: usize = 1;
pub const IPC_STAT: usize = 2;
// shm-specific cmds
pub const SHM_LOCK: usize = 11;
pub const SHM_UNLOCK: usize = 12;

// for shmat
pub const SHM_RDONLY: usize = 0o10000;
pub const SHM_RND: usize = 0o20000;
pub const SHM_REMAP: usize = 0o40000;
pub const SHM_EXEC: usize = 0o100000;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IpcPerm {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub mode: u16,
    _pad1: u16,
    _seq: u16,
    _pad2: u16,
    _unused1: usize,
    _unused2: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ShmIdDs {
    pub shm_perm: IpcPerm,
    pub shm_segsz: usize,
    pub shm_atime: usize, // time_t
    pub shm_dtime: usize, // time_t
    pub shm_ctime: usize, // time_t
    pub shm_cpid: u32,
    pub shm_lpid: u32,
    pub shm_nattch: usize,
    _unused4: usize,
    _unused5: usize,
}
impl Default for ShmIdDs {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
/// 内核中表示一个共享内存段
pub struct SharedMemorySegment {
    pub id_ds: ShmIdDs,
    pub frames: Vec<Arc<FrameTracker>>,
    pub marked_for_deletion: bool,
}

impl SharedMemorySegment {
    pub fn new(key: i32, size: usize, pid: usize) -> Option<Self> {
        let num_pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        // 分配连续的物理页
        let frames = frame_alloc_continuous(num_pages)?;

        let creator_uid = 0; // 简化：假设为 root
        let creator_gid = 0; // 简化：假设为 root

        let id_ds = ShmIdDs {
            shm_perm: IpcPerm {
                key,
                uid: creator_uid,
                gid: creator_gid,
                cuid: creator_uid,
                cgid: creator_gid,
                mode: 0o666, // 默认权限 rw-rw-rw-
                _pad1: 0,
                _seq: 0,
                _pad2: 0,
                _unused1: 0,
                _unused2: 0,
            },
            shm_segsz: size,
            shm_atime: 0,
            shm_dtime: 0,
            shm_ctime: crate::timer::get_time(), // 创建时间
            shm_cpid: pid as u32,
            shm_lpid: 0,
            shm_nattch: 0,
            _unused4: 0,
            _unused5: 0,
        };

        Some(Self {
            id_ds,
            frames,
            marked_for_deletion: false,
        })
    }

    pub fn page_count(&self) -> usize {
        self.frames.len()
    }

    pub fn get_frame_ppn(&self, page_index: usize) -> Option<PhysPageNum> {
        self.frames.get(page_index).map(|frame| frame.ppn)
    }

    pub fn attach(&mut self, attacher_pid: u32) {
        self.id_ds.shm_nattch += 1;
        self.id_ds.shm_lpid = attacher_pid;
        self.id_ds.shm_atime = crate::timer::get_time();
    }

    pub fn detach(&mut self, detacher_pid: u32) {
        if self.id_ds.shm_nattch > 0 {
            self.id_ds.shm_nattch -= 1;
        }
        self.id_ds.shm_lpid = detacher_pid;
        self.id_ds.shm_dtime = crate::timer::get_time();
    }

    pub fn is_deletable(&self) -> bool {
        self.marked_for_deletion && self.id_ds.shm_nattch == 0
    }
}

/// 全局共享内存管理器
pub struct ShmManager {
    // key -> shmid
    pub key_to_id: BTreeMap<i32, i32>,
    // shmid -> segment
    pub id_to_segment: BTreeMap<i32, Arc<Mutex<SharedMemorySegment>>>,
    // 用于生成唯一的 shmid
    pub next_id: AtomicUsize,
}

impl ShmManager {
    pub fn new() -> Self {
        Self {
            key_to_id: BTreeMap::new(),
            id_to_segment: BTreeMap::new(),
            next_id: AtomicUsize::new(0),
        }
    }

}

pub static SHM_MANAGER: LazyInit<Mutex<ShmManager>> = LazyInit::new();
pub fn init_shm() {
    SHM_MANAGER.init_by(Mutex::new(ShmManager::new()));
}
