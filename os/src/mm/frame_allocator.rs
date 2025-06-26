// os/src/mm/frame_allocator.rs

use crate::config::{PAGE_SIZE, PAGE_SIZE_BITS};
use crate::mm::PhysPageNum;
use crate::sync::UPSafeCell;
use crate::task::current_task_may_uninit;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};
use bit_field::{BitArray, BitField};
use lazy_static::lazy_static;
use log::{error, info};
use spin::Mutex; // 假设您的 Mutex 在这里
use crate::mm::{PhysAddr}; // 假设您的 PhysAddr 和 pa! 宏在这里

// --- 1. FrameTracker 定义（保持您的版本） ---
//    我们保留您的 FrameTracker，因为它与您的系统其他部分（如 Drop 逻辑）紧密耦合。

#[derive(Clone)]
pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    pub fn ppn(&self) -> PhysPageNum {
        self.ppn
    }
    
    // 从 PhysPageNum 创建一个新的 FrameTracker
    // 这个方法现在更重要了
    pub fn new(ppn: PhysPageNum) -> Self {
        let bytes_array = ppn.get_bytes_array();
        for i in bytes_array {

        println!("i:{:#x}",i as *const _ as usize);
            *i = 0;
        }
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("FrameTracker:PPN={:#x}", self.ppn.0))
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

/// 页帧分布图
/// 利用位图管理一个连续的物理内存区域
#[derive(Debug)]
struct FrameRegionMap {
    bits: Vec<usize>,
    start_ppn: usize,
    end_ppn: usize,
}

impl FrameRegionMap {
    /// 创建页帧分布图
    fn new(start_paddr: PhysAddr, end_paddr: PhysAddr) -> Self {
        let start_ppn = start_paddr.floor().0;
        let end_ppn = end_paddr.floor().0;
        let num_pages = end_ppn - start_ppn;
        // 使用 .div_ceil() 来确保分配足够的 usize 来存储所有位
        let mut bits = alloc::vec![0usize; num_pages.div_ceil(usize::BITS as usize)];

        // 将超出实际内存范围的位标记为已使用，防止分配到不存在的内存
        for i in num_pages..(bits.len() * usize::BITS as usize) {
            bits.set_bit(i, true);
        }

        Self {
            bits,
            start_ppn,
            end_ppn,
        }
    }

    /// 申请一个空闲页
    fn alloc(&mut self, _policy: usize) -> Option<PhysPageNum> {
        // TODO: policy 可以用来决定是从头找还是从尾找，这里暂时只实现一种
        for i in 0..self.bits.len() {
            if self.bits[i] != usize::MAX { // 如果这个 usize 块中还有空位
                for bit_index in 0..usize::BITS as usize {
                    if !self.bits[i].get_bit(bit_index) {
                        self.bits[i].set_bit(bit_index, true);
                        let page_offset = i * (usize::BITS as usize) + bit_index;
                        return Some(PhysPageNum(self.start_ppn + page_offset));
                    }
                }
            }
        }
        None
    }

    /// 申请多个连续的空闲页
    fn alloc_contiguous(&mut self, count: usize, _policy: usize) -> Option<Vec<FrameTracker>> {
        if count == 0 { return Some(Vec::new()); }
        
        let num_pages = self.end_ppn - self.start_ppn;
        if count > num_pages { return None; }
        
        let mut found_start = 0;
        let mut consecutive_free = 0;

        for i in 0..num_pages {
            if !self.bits.get_bit(i) {
                if consecutive_free == 0 {
                    found_start = i;
                }
                consecutive_free += 1;
                if consecutive_free == count {
                    // 找到了！标记并分配
                    let mut frames = Vec::with_capacity(count);
                    for j in 0..count {
                        let bit_index = found_start + j;
                        self.bits.set_bit(bit_index, true);
                        frames.push(FrameTracker::new(PhysPageNum(self.start_ppn + bit_index)));
                    }
                    return Some(frames);
                }
            } else {
                consecutive_free = 0;
            }
        }
        None // 没有找到足够大的连续块
    }

    /// 释放一个已经使用的页
    fn dealloc(&mut self, ppn: PhysPageNum) {
        let bit_index = ppn.0 - self.start_ppn;
        if self.bits.get_bit(bit_index) {
            self.bits.set_bit(bit_index, false);
        } else {
            panic!("Deallocating a frame that was not allocated: PPN {:#x}", ppn.0);
        }
    }
    
    /// 获取空闲页数量
    fn free_pages(&self) -> usize {
        self.bits.iter().map(|&word| word.count_zeros() as usize).sum()
    }
}

/// 一个总的页帧分配器，可以管理多个不连续的内存区域
#[derive(Debug)]
pub struct FrameAllocator {
    regions: Vec<FrameRegionMap>,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// 添加一块可供分配的物理内存区域
    pub fn add_memory_region(&mut self, start_paddr: PhysAddr, end_paddr: PhysAddr) {
        info!(
            "FrameAllocator: adding region [{:#x}, {:#x})",
            start_paddr.0, end_paddr.0
        );
        self.regions.push(FrameRegionMap::new(start_paddr, end_paddr));
    }

    pub fn alloc(&mut self, policy: usize) -> Option<PhysPageNum> {
        for region in &mut self.regions {
            if let Some(ppn) = region.alloc(policy) {
                return Some(ppn);
            }
        }
        None
    }

    pub fn dealloc(&mut self, ppn: PhysPageNum) {
        for region in &mut self.regions {
            if ppn.0 >= region.start_ppn && ppn.0 < region.end_ppn {
                region.dealloc(ppn);
                return;
            }
        }
        panic!("Deallocating a frame in an unknown memory region: PPN {:#x}", ppn.0);
    }
    
    pub fn alloc_contiguous(&mut self, count: usize, policy: usize) -> Option<Vec<FrameTracker>> {
        for region in &mut self.regions {
            if let Some(frames) = region.alloc_contiguous(count, policy) {
                return Some(frames);
            }
        }
        None
    }

    pub fn remaining_frames(&self) -> usize {
        self.regions.iter().map(|r| r.free_pages()).sum()
    }
}

// --- 3. 全局实例和公共 API (保持您的接口) ---

lazy_static! {
        /// 用于持久化分配的全局 FrameTracker 存储。
    /// Key 是物理页号 (PPN)，Value 是一个 Arc，用于阻止 FrameTracker 被 drop。
    pub static ref FRAME_VEC: Mutex<BTreeMap<usize, Arc<FrameTracker>>> =
        Mutex::new(BTreeMap::new());
    /// 全局物理帧分配器实例
    pub static ref FRAME_ALLOCATOR: Mutex<FrameAllocator> =
         Mutex::new(FrameAllocator::new());
}

/// 初始化帧分配器，由外部调用者负责添加内存区域
pub fn init_frame_allocator() {
    // 这里的逻辑现在由外部的 add_memory_region 调用来完成
    // 不再需要在这里计算 ekernel 等地址
    // 我们可以加一个检查，确保至少有一个区域被添加了
    extern "C" {
        fn ekernel();
    }
    // let ekernel_paddr = PhysAddr::from(ekernel as usize);
    // // 这里用一个示例来添加内存区域，你需要根据你的平台来修改
    // add_memory_region(ekernel_paddr, PhysAddr(crate::config::MEMORY_END));
}

/// 添加一块可供分配的物理内存区域 (来自 ByteOS 的接口)
pub fn add_memory_region(start_paddr: PhysAddr, end_paddr: PhysAddr) {

    FRAME_ALLOCATOR.lock().add_memory_region(start_paddr, end_paddr);
    info!(
        "Total free pages: {}",
        FRAME_ALLOCATOR.lock().remaining_frames()
    );
}

/// 分配单个物理页帧 (保持您的接口)
pub fn frame_alloc() -> Option<Arc<FrameTracker>> {
    let policy = match current_task_may_uninit() {
        Some(task) => task.get_noma_policy(),
        None => 0,
    };
    FRAME_ALLOCATOR
        .lock()
        .alloc(policy)
        .map(FrameTracker::new)
        .map(Arc::new)
}

/// 释放单个物理页帧 (保持您的接口)
pub fn frame_dealloc(ppn: PhysPageNum) {
    FRAME_ALLOCATOR.lock().dealloc(ppn);
}

/// 分配多个连续的物理页帧 (适配您的接口)
pub fn frame_alloc_continue(count: usize) -> Option<Vec<Arc<FrameTracker>>> {
    let policy = match current_task_may_uninit() {
        Some(task) => task.get_noma_policy(),
        None => 0,
    };

    FRAME_ALLOCATOR
        .lock()
        .alloc_contiguous(count, policy)
        .map(|vec| vec.into_iter().map(Arc::new).collect())
}

/// 获取剩余页数 (这是新的、来自 ByteOS 的功能)
pub fn remaining_frames() -> usize {
    FRAME_ALLOCATOR.lock().remaining_frames()
}

/// 申请一个持久化存在的物理页，它不会被自动回收。
/// 返回的是物理地址。
pub fn frame_alloc_persist() -> Option<usize> {
    // 1. 调用常规的 frame_alloc 获取一个被 Arc 管理的 FrameTracker
    frame_alloc().map(|frame_arc| {
        // 2. 获取物理页号和物理地址
        let ppn = frame_arc.ppn().0;
        let paddr = ppn << PAGE_SIZE_BITS;
        
        // 3. 将 Arc 存入全局的 BTreeMap 中，这会增加引用计数，
        //    即使外层的 frame_arc 在函数结束时被 drop，
        //    BTreeMap 中的 Arc 仍然存在，所以 FrameTracker 不会被 drop。
        FRAME_VEC.lock().insert(ppn, frame_arc);
        
        // 4. 返回物理地址
        paddr
    })
}

/// 手动释放一个之前通过 frame_alloc_persist 分配的物理页。
/// `paddr` 必须是之前 `frame_alloc_persist` 返回的物理地址。
pub fn frame_dealloc_persist(paddr: usize) {
    let ppn = paddr >> PAGE_SIZE_BITS;
    
    // 从 BTreeMap 中移除对应的 Arc。
    // 如果移除成功，并且这是最后一个对 FrameTracker 的强引用，
    // Arc 会被 drop，接着 FrameTracker 也会被 drop，
    // 最终触发 frame_dealloc，完成物理页的回收。
    if FRAME_VEC.lock().remove(&ppn).is_none() {
        // 如果在 map 中找不到，说明这个地址可能没有被持久化分配，或者被重复释放了
        panic!("frame_dealloc_persist: address {:#x} (PPN {:#x}) was not allocated persistently or already deallocated.", paddr, ppn);
    }
}




#[allow(unused)]
/// a simple test for frame allocator
pub fn frame_allocator_test() {
    let mut v: Vec<Arc<FrameTracker>> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:#?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:#?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}
