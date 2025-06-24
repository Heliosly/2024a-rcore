use crate::{mm::{get_target_ref, shm::{ShmIdDs, SHM_LOCK, SHM_UNLOCK}, VirtAddr}, task::current_process, utils::error::{SysErrNo, SyscallRet}};
use crate::task::ProcessControlBlock;
pub async  fn sys_shmat(shmid: i32, shmaddr: usize, shmflg: usize) -> SyscallRet {
    // 1. 获取共享段
    let segment_arc = {
        let manager = SHM_MANAGER.lock().await;
        match manager.id_to_segment.get(&shmid) {
            Some(arc) => arc.clone(),
            None => return Err(SysErrNo::EINVAL),
        }
    };
    
    // 2. 准备映射
    let process = current_process();
    let mut ms = process.memory_set.lock();
    let segment = segment_arc.lock(); // 临时锁住以读取信息

    let page_count = segment.page_count();
    if page_count == 0 {
        return Ok(shmaddr); 
    }
    

    // 3. 分配虚拟地址空间 (与之前相同)
    let start_vpn = ms.areatree.alloc_pages_from_hint(page_count, VirtAddr::from(shmaddr).ceil())
        .ok_or(SysErrNo::ENOMEM)?;
    let end_vpn = VirtPageNum(start_vpn.0 + page_count);

    // 4. 创建一个 SharedMemory 类型的 MapArea
    let perm = MapPermission::U | MapPermission::R | MapPermission::W; // 根据 shmflg 设置
    let  map_area = MapArea::new_by_vpn(
        start_vpn,
        end_vpn,
        MapType::Framed, 
        
        perm,
        MapAreaType::Shm{shmid}
    );

    // 5. 将共享段的物理帧逐个映射到 MapArea 的虚拟地址范围
    let map: BTreeMap<VirtPageNum, Arc<FrameTracker>> = segment
    .frames
    .iter()
    .enumerate()
    .map(|(i, frame)| {
        (VirtPageNum(start_vpn.0 + i), (*frame).clone())
    })
    .collect();
    ms.push_with_given_frames(map_area, &map, false);

   
    drop(segment); 
    segment_arc.lock().attach(process.pid.0 as u32);
    
    Ok(VirtAddr::from(start_vpn).0)
}


pub async fn sys_shmget(key: i32, size: usize, shmflg: usize) -> SyscallRet {
    let mut manager = SHM_MANAGER.lock().await;

    // 校验 size
    if size > crate::config::MAX_SHM_SIZE { // 检查是否超过系统限制
        return Err(SysErrNo::EINVAL);
    }

    let is_create = (shmflg & crate::mm::shm::IPC_CREAT) != 0;
    let is_exclusive = (shmflg & crate::mm::shm::IPC_EXCL) != 0;

    // --- 情况 1: 查找一个已存在的段 (当 key 不是 IPC_PRIVATE) ---
    if key != crate::mm::shm::IPC_PRIVATE {
        if let Some(&shmid) = manager.key_to_id.get(&key) {
            // 键已存在
            if is_create && is_exclusive {
                // 如果同时指定了 IPC_CREAT 和 IPC_EXCL，但键已存在，则返回错误
                return Err(SysErrNo::EEXIST);
            }
            
            let segment_arc = manager.id_to_segment.get(&shmid).unwrap();
            let segment = segment_arc.lock();
            
            // 检查请求的大小是否超过了已存在段的大小
            if size > segment.id_ds.shm_segsz {
                return Err(SysErrNo::EINVAL);
            }
            // TODO: 一个完整的实现还需要在这里检查权限

            return Ok(shmid as usize);
        }
    }

    // --- 情况 2: 创建一个新的段 ---
    // 如果 key 不存在，但用户又没有指定 IPC_CREAT，则返回错误
    if !is_create && key != crate::mm::shm::IPC_PRIVATE {
        return Err(SysErrNo::ENOENT);
    }
    // 不能创建一个大小为 0 的段
    if size == 0 {
        return Err(SysErrNo::EINVAL);
    }
    
    let pid = current_process().pid.0;
    let new_segment = match SharedMemorySegment::new(key, size, pid) {
        Some(seg) => seg,
        None => return Err(SysErrNo::ENOMEM), // 物理帧分配失败
    };
   
    // 分配一个新的、唯一的 shmid
    let shmid = manager.next_id.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as i32;
    let segment_arc = Arc::new(crate::sync::Mutex::new(new_segment));

    // 将新段加入全局管理器
    manager.id_to_segment.insert(shmid, segment_arc);
    if key != crate::mm::shm::IPC_PRIVATE {
        manager.key_to_id.insert(key, shmid);
    }

    Ok(shmid as usize)
}

/// 功能: 控制一个共享内存段 (shmctl)
pub async fn sys_shmctl(shmid: i32, cmd: usize, buf: *mut ShmIdDs) -> SyscallRet {
    let mut manager = crate::mm::shm::SHM_MANAGER.lock().await;
    
    let pcb = current_process();
    let token = pcb.get_user_token().await;

    pcb.manual_alloc_type_for_lazy(buf).await?;

    let segment_arc = match manager.id_to_segment.get(&shmid) {
        Some(arc) => arc.clone(),
        None => return Err(SysErrNo::EINVAL),
    };

    let mut segment = segment_arc.lock();

    match cmd {
        crate::mm::shm::IPC_STAT => {
            if buf.is_null() || crate::mm::put_data(token, buf, segment.id_ds).is_err() {
                return Err(SysErrNo::EFAULT);
            }
            Ok(0)
        }
        crate::mm::shm::IPC_SET => {
            if buf.is_null() {
                return Err(SysErrNo::EFAULT);
            }
            let user_ds = *get_target_ref(token, buf)?;
            segment.id_ds.shm_perm.uid = user_ds.shm_perm.uid;
            segment.id_ds.shm_perm.gid = user_ds.shm_perm.gid;
            segment.id_ds.shm_perm.mode = user_ds.shm_perm.mode & 0o777;
            segment.id_ds.shm_ctime = crate::timer::get_time();
            Ok(0)
        }
        crate::mm::shm::IPC_RMID => {
            segment.marked_for_deletion = true;
            let key_to_remove = segment.id_ds.shm_perm.key;

            if segment.is_deletable() {
                if key_to_remove != crate::mm::shm::IPC_PRIVATE {
                    manager.key_to_id.remove(&key_to_remove);
                }
                manager.id_to_segment.remove(&shmid);
            }
            Ok(0)
        }
        SHM_LOCK | SHM_UNLOCK => Ok(0),
        _ => Err(SysErrNo::EINVAL),
    }
}

/// 功能: 分离一个共享内存段 (shmdt)
pub async fn sys_shmdt(shmaddr: usize) -> SyscallRet {
    let vaddr = VirtAddr::from(shmaddr);
    let process = current_process();

    // 根据虚拟地址找到包含它的 MapArea
    // TODO: 完成逻辑
    Ok(0)
}