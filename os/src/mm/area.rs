use core::{
    cmp::{max, min},
    ops::{Deref, DerefMut, Range},
};

use alloc::{collections::btree_map::BTreeMap, sync::Arc, vec::Vec};
use linux_raw_sys::general::SEEK_CUR;
use lwext4_rust::bindings::SEEK_SET;

use crate::{
    config::{KERNEL_PGNUM_OFFSET, MMAP_BASE, MMAP_PGNUM_TOP, MMAP_TOP, PAGE_SIZE, PAGE_SIZE_BITS, USER_STACK_TOP},
    fs::{File, FileDescriptor, OsInode},
    mm::StepByOne,
    syscall::flags::MmapProt,
    utils::error::{GeneralRet, SyscallRet, TemplateRet},
};

use super::{
    flush_tlb, frame_alloc, page_table::PTEFlags, FrameTracker, PageTable, PhysPageNum, VPNRange, VirtAddr, VirtPageNum
};

/// 虚拟内存区域树：Key 按照 Range.start_vpn 排序
pub struct VmAreaTree {
    pub areas: BTreeMap<VirtPageNum, MapArea>,
}
impl Deref for VmAreaTree {
    type Target = BTreeMap<VirtPageNum, MapArea>;

    fn deref(&self) -> &Self::Target {
        &self.areas
    }
}

impl DerefMut for VmAreaTree {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.areas
    }
}
impl VmAreaTree {
    pub fn push(&mut self, area: MapArea) {
        // trace!("[VmAreaTree] push area: l:{:#x} r:{:#x}",area.vpn_range.get_start().0,area.vpn_range.get_end().0 );


        self.areas.insert(area.vpn_range.get_start(), area);
    }
    /// 创建一个空的树
    pub fn new() -> Self {
        VmAreaTree {
            areas: BTreeMap::new(),
        }
    }

    pub fn remove_by_area(&mut self, area: MapArea) -> MapArea {
        self.areas.remove(&area.vpn_range.get_start()).unwrap()
    }
    /// 手动插入一段页号区间，覆盖前需保证不重叠
    pub fn insert_vpn_area(
        &mut self,
        start_vpn: VirtPageNum,
        end_vpn: VirtPageNum,
        map_type: MapType,
        perm: MapPermission,
        area_type: MapAreaType,
    ) -> Result<(), &'static str> {
        let new_range = start_vpn..end_vpn;
        if self.is_overlap(&new_range) {
            return Err("insert: overlap with existing region");
        }
        let area = MapArea::new_by_vpn(start_vpn, end_vpn, map_type, perm, area_type);
        self.areas.insert(start_vpn, area);
        Ok(())
    }
    pub fn alloc_pages_from_hint(
        &self, 
        npages: usize,
        hint_vpn: VirtPageNum,
    ) -> Option<VirtPageNum> {
        // --- 1. 优先从 hint_vpn 开始向后查找 ---

        // 规范化 hint_vpn，确保它在合法范围内
        let start_hint_vpn = max(hint_vpn, VirtPageNum::from(MMAP_BASE>>PAGE_SIZE_BITS));

        // BTreeMap::range(start..) 可以高效地找到第一个键 >= start 的项。
        // 我们用它来定位 hint 之后的所有已分配区域。
        let mut last_area_end_vpn = start_hint_vpn;

        for (area_start_vpn, area) in self.areas.range(start_hint_vpn..) {
            // 我们要找的空闲区域是 [last_area_end_vpn, area_start_vpn)
            // 检查这个“空洞”是否足够大
            let gap_start_vpn = last_area_end_vpn;
            let gap_end_vpn = *area_start_vpn;
            
            if gap_end_vpn.0 > gap_start_vpn.0 && // 确保空洞存在
               gap_end_vpn.0 - gap_start_vpn.0 >= npages
            {
                // 找到了！这个空洞足够大。
                // 我们可以在这个空洞的起始处分配。
                // 同时要确保分配后不会超过 MMAP_TOP
                if gap_start_vpn.0 + npages <= MMAP_TOP / PAGE_SIZE {
                    return Some(gap_start_vpn);
                }
            }

            // 更新 last_area_end_vpn，为下一次循环做准备
            last_area_end_vpn = max(last_area_end_vpn, area.vpn_range.get_end());
        }

        // 如果循环结束后，最后的区域到 MMAP_TOP 之间还有空间，也检查一下
        if MMAP_TOP / PAGE_SIZE > last_area_end_vpn.0 &&
           MMAP_TOP / PAGE_SIZE - last_area_end_vpn.0 >= npages
        {
            return Some(last_area_end_vpn);
        }

        // --- 2. 如果从 hint 开始找不到，则回退到全局搜索 ---
        //    这里的逻辑就是你之前的 alloc_pages，但我们在这里重新实现
        //    它，避免在 sys_mmap 中写复杂的 if-else。

        // 从 MMAP_TOP 向下搜索
        let mut prev_area_start_vpn = VirtPageNum(MMAP_TOP / PAGE_SIZE);

        for area in self.areas.values().rev() {
            let current_area_end_vpn = area.vpn_range.get_end();
            
            // 空洞是 [current_area_end_vpn, prev_area_start_vpn)
            if prev_area_start_vpn.0 > current_area_end_vpn.0 &&
               prev_area_start_vpn.0 - current_area_end_vpn.0 >= npages
            {
                // 找到了一个足够大的空洞，从空洞的末尾开始分配
                let alloc_start_vpn = VirtPageNum(prev_area_start_vpn.0 - npages);
                return Some(alloc_start_vpn);
            }
            prev_area_start_vpn = area.vpn_range.get_start();
        }

        // 检查最开始的一段区域 [USER_STACK_TOP, 第一个area的start)
        let final_gap_end = prev_area_start_vpn;
        let final_gap_start = VirtPageNum::from(MMAP_BASE>>PAGE_SIZE_BITS);
        if final_gap_end.0 > final_gap_start.0 &&
           final_gap_end.0 - final_gap_start.0 >= npages
        {
             // 从空洞的末尾开始分配
             let alloc_start_vpn = VirtPageNum(final_gap_end.0 - npages);
             return Some(alloc_start_vpn);
        }

        None
    }
    /// 从 MMAP_PGNUM_TOP 向下找 npages 个连续页号，插入并返回起始页号
    pub fn alloc_pages(&mut self, npages: usize) -> Option<VirtPageNum> {
        let mut end_pn = MMAP_PGNUM_TOP;

        for area in self.areas.values().rev() {
            let s = area.vpn_range.get_start().0;
            let e = area.vpn_range.get_end().0;

            // gap = [e, end_pn)
            if end_pn >= e + npages {
                let start_pn = end_pn - npages;
                if start_pn >= e {
                    let start_vpn = VirtPageNum(start_pn);

                    return Some(start_vpn);
                }
            }
            end_pn = min(end_pn, s);
        }

        // 最后，如果 [0, end_pn) 足够，也直接分配
        if end_pn >= npages {
            let start_pn = end_pn - npages;
            let start_vpn = VirtPageNum(start_pn);

            return Some(start_vpn);
        }

        None
    }

    /// 按起始页号释放该区域
    ///
    fn dealloc(&mut self, start_vpn: VirtPageNum) -> Option<MapArea> {
        match self.areas.remove(&start_vpn) {
            Some(area) => Some(area),

            None => {
                println!("dealloc: no such region");
                None
            }
        }
    }

    pub fn is_fully_contained(&self, range: &Range<VirtPageNum>) -> bool {
        let mut cursor = range.start.0;
        let end = range.end.0;
    
        while cursor < end {
            // 找到最后一个 vpn_range.start <= cursor 的 area
            let entry = self.areas.range(..=VirtPageNum(cursor)).next_back();
            let (_, area) = match entry {
                Some(e) => e,
                None => return false, // 没有任何区域覆盖 cursor
            };
    
            let a_start = area.vpn_range.get_start().0;
            let a_end = area.vpn_range.get_end().0;
    
            // ️注意是左闭右开：如果 cursor < a_start，说明有空洞；如果 cursor >= a_end，同样不在当前区间
            if cursor < a_start || cursor >= a_end {
                return false;
            }
    
            // 推进 cursor 到当前区域结尾
            cursor = a_end;
        }
    
        // 成功走完所有 cursor，说明整段 range 都被覆盖
        true
    }
    
    /// 检查一个页号区间是否与已存在任何区域重叠
    pub fn is_overlap(&self, range: &Range<VirtPageNum>) -> bool {
        for area in self.areas.values() {
            let a_s = area.vpn_range.get_start().0;
            let a_e = area.vpn_range.get_end().0;
            if range.start.0 < a_e && a_s < range.end.0 {
                return true;
            }
        }
        false
    }
    ///使用一个vpn去找对应存在的area，返回area的start_va
    pub fn find_area(&self, vpn: VirtPageNum) -> Option<VirtPageNum> {
        // 将虚拟地址转换为页号
        // 在BTreeMap中查找最后一个起始页号 <= 当前页号的区域
        self.areas
            .range(..=vpn)
            .next_back()
            .and_then(|(viddr, area)| {

                // 检查该区域是否包含目标页号
                if area.contains(vpn) {

                    Some(*viddr)
                } else {
                    

                
                    None
                }
            })
    }
    /// 从 `hint` 向下寻找能容纳 `npages` 页的第一个空闲区
    pub fn find_gap_from(&self, hint: VirtPageNum, npages: usize) -> Option<VirtPageNum> {
        let mut end_pn = hint.0;

        // 遍历所有 start < hint 的区域，倒序（从高地址到低地址）
        for (start, area) in self.areas.range(..hint).rev() {
            let s = start.0;
            let e = area.vpn_range.get_end().0;
            // 检查 gap = [e, end_pn) 是否足够大
            if end_pn >= e + npages {
                let start_pn = end_pn - npages;
                if start_pn >= e {
                    // 找到符合条件的 gap，返回起始页号
                    return Some(VirtPageNum(start_pn));
                }
            }
            // 否则收缩搜索上界到本区域的起始页号
            end_pn = min(end_pn, s);
        }
        // 最后，如果从 0 到 end_pn 足够，也返回
        if end_pn >= npages {
            return Some(VirtPageNum(end_pn - npages));
        }
        None
    }
    pub fn debug_print(&self) {
        for (start, area) in &self.areas {
            let end = area.vpn_range.get_end();
            println!(
                "VmArea [{:#x}..{:#x}) type={:?} perm={:?} area_type={:?}",
                start.0, end.0, area.map_type, area.map_perm, area.area_type
            );
        }
    }
}
bitflags! {
    /// Mmap flags
    pub struct MmapFlags: u32 {
        /// Share changes
        const MAP_SHARED = 1;
        /// Changes are private
        const MAP_PRIVATE = 1 << 1;
        /// Interpret addr exactly
        const MAP_FIXED = 1 << 4;
        /// Don't use a file
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't permit write
        const MAP_DENYWRITE = 1 << 11;
        /// Populate (prefault) page tables
        const MAP_POPULATE = 1 << 13;
        /// Region grows down (like a stack)
        const MAP_STACK = 1 << 17;
        /// MAP_FIXED_NOREPLACE: Don't replace existing mapping
        const MAP_FIXED_NOREPLACE = 1 << 16;
        /// Don't reserve swap space
        const MAP_NORESERVE = 1 << 14; // 0x4000
    }
}

/// map area structure, controls a contiguous piece of virtual memory
pub struct MapArea {
    ///从start到end的vpn
    pub vpn_range: VPNRange,
    pub data_frames: BTreeMap<VirtPageNum, Arc<FrameTracker>>,
    pub map_type: MapType,
    pub map_perm: MapPermission,
    pub area_type: MapAreaType,
    ///只有Osinnoder才能映射
    pub fd: Option<MmapFile>,
    pub mmap_flags: MmapFlags,
}

impl MapArea {
    pub fn range_size(&self)->usize{
        (self.vpn_range.get_end().0-self.vpn_range.get_start().0)<<PAGE_SIZE_BITS
    }
    ///返回已经映射过的 页大小
    pub fn size(&self) -> usize {
        self.data_frames.len() * PAGE_SIZE
    }
    pub fn debug_print(&self) {
        let end = self.vpn_range.get_end();
        let start = self.vpn_range.get_start();
        println!(
            "Area [{:#x}..{:#x}) type={:?} perm={:?} map_type={:?},map_area_type={:?}",
            start.0, end.0, self.map_type, self.map_perm, self.map_type, self.area_type
        );
    }
    pub fn allocated(&self, vpn: VirtPageNum) -> bool {
        self.data_frames.contains_key(&vpn)
    }
    pub fn set_fd(&mut self, fd: Option<MmapFile>) {
        self.fd = fd;
    }
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
        area_type: MapAreaType,
    ) -> Self {
        let start_vpn: VirtPageNum = start_va.floor();
        let end_vpn: VirtPageNum = end_va.ceil();
        Self {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
            area_type,
            fd: None,
            mmap_flags: MmapFlags::empty(),
        }
    }
    pub fn new_by_vpn(
        start_vpn: VirtPageNum,
        end_vpn: VirtPageNum,
        map_type: MapType,
        map_perm: MapPermission,
        area_type: MapAreaType,
    ) -> Self {
        Self {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
            area_type,
            fd: None,
            mmap_flags: MmapFlags::empty(),
        }
    }
    pub fn from_another(another: &Self) -> Self {
        Self {
            vpn_range: VPNRange::new(another.vpn_range.get_start(), another.vpn_range.get_end()),
            data_frames: BTreeMap::new(),
            map_type: another.map_type,
            map_perm: another.map_perm,
            area_type: another.area_type,
            fd: another.fd.clone(),

            mmap_flags: MmapFlags::empty(),
        }
    }
       /// Map each provided frame to its corresponding VPN.
    ///
    /// # Arguments
    ///
    /// * `page_table` – 目标页表
    /// * `frames` – 一个 BTreeMap，键是要映射的 VirtPageNum，值是相应的 FrameTracker
    pub fn map_given_frames(
        &mut self,
        page_table: &mut PageTable,
        frames: &BTreeMap<VirtPageNum, Arc<FrameTracker>>,
        is_cow:bool,
    ) {
        // 先获取 PTE flags
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits as usize)
            .expect("Invalid permission flags");

        // 对每对 (vpn, frame) 做映射并记录
        for (vpn, frame) in frames {
            // println!("[MapArea] mapping vpn:{:#x} to frame ppn:{:#x} cow :{:?}", vpn.0, frame.ppn().0,is_cow);
            // 将这条映射插入页表
            page_table.map(*vpn, frame.ppn(), pte_flags);
            if is_cow {
                // 如果是 COW 映射，设置相应的标志
              let   pte=  page_table.find_pte(*vpn).unwrap();
                pte.set_cow();
                // println!("pte:{:?}",pte.flags());
                // 记录到 data_frames 中

            self.data_frames.insert(*vpn, frame.clone());
            }
        }
    }
    /// Update area's mapping flags and write it to page table. You need to flush TLB after calling
    /// this function.
    pub fn update_flags(&mut self, flags: MapPermission, page_table: &mut PageTable) {
        self.map_perm = flags;
        page_table
            .update_region(self.vpn_range.get_start().into(), self.size(), flags)
            .unwrap();
    }
    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) ->GeneralRet {
        let ppn: PhysPageNum;

        match self.map_type {
            MapType::Framed => {
                
                let frame = match frame_alloc(){
                    Some(f) => f,
                    None => return Err(crate::utils::error::SysErrNo::ENOMEM),
                };
                ppn = frame.ppn();

                self.data_frames.insert(vpn, frame);
            }
            MapType::Direct => {
                ppn = PhysPageNum(vpn.0 - KERNEL_PGNUM_OFFSET);
            }
        }
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits as usize).unwrap();
        page_table.map(vpn, ppn, pte_flags);
        Ok(())
    }
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if self.map_type == MapType::Framed {
            self.data_frames.remove(&vpn);
        }
        page_table.unmap(vpn);
    }

    pub fn map(&mut self, page_table: &mut PageTable)->GeneralRet {
        for vpn in self.vpn_range {
            self.map_one(page_table, vpn)?
        }
        Ok(())
    }
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.unmap_one(page_table, vpn);
        }
    }
    #[allow(unused)]
    pub fn shrink_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(new_end, self.vpn_range.get_end()) {
            self.unmap_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }
    #[allow(unused)]
    pub fn append_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum)->GeneralRet {
        for vpn in VPNRange::new(self.vpn_range.get_end(), new_end) {
            self.map_one(page_table, vpn)?
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
        Ok(())
    }
    /// data: start-aligned but maybe with shorter length
    /// assume that all frames were cleared before
    pub fn copy_data(&mut self, page_table: &mut PageTable, data: &[u8], offset: usize) {
        assert_eq!(self.map_type, MapType::Framed);
        assert!(offset < PAGE_SIZE);
        let mut start: usize = 0;
        let mut current_vpn = self.vpn_range.get_start();
        let len = data.len();

        let mut page_offset = offset;
        loop {
            let src = &data[start..len.min(start + PAGE_SIZE - page_offset)];
            let dst = &mut page_table
                .translate(current_vpn)
                .unwrap()
                .ppn()
                .get_bytes_array()[page_offset..(page_offset + src.len())];
            dst.copy_from_slice(src);

            start += PAGE_SIZE - page_offset;

            page_offset = 0;
            if start >= len {
                break;
            }
            current_vpn.step();
        }
    }
    /// 克隆当前 MapArea，但只保留 new_range 范围内的页号和对应的 data_frames。
    pub fn from_another_with_range(&self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> MapArea {
        Self {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type: self.map_type,
            map_perm: self.map_perm,
            area_type: self.area_type,
            fd: None,

            mmap_flags: MmapFlags::empty(),
        }
    }
    pub fn start_vpn(&self) -> VirtPageNum {
        self.vpn_range.get_start()
    }
    pub fn end_vpn(&self) -> VirtPageNum {
        self.vpn_range.get_end()
    }
    ///判断是否包含所给地址
    pub fn contains(&self, vpn: VirtPageNum) -> bool {
        self.vpn_range.contains(vpn)
    }
    ///判断是否与所给地址段是否有交集
    pub fn overlap_with(&self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> bool {
        self.start_vpn() <= start_vpn && start_vpn < self.end_vpn()
            || start_vpn <= self.start_vpn() && self.start_vpn() < end_vpn
    }

    /// 判断是否包含所给地址段.
    pub fn contained_in(&self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> bool {
        start_vpn <= self.start_vpn() && self.end_vpn() <= end_vpn
    }
    ///判断是否严格包含(真包含)所给地址段
    pub fn strict_contain(&self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> bool {
        self.start_vpn() < start_vpn && end_vpn < self.end_vpn()
    }

    /// 将一个 area分割为三段，其中|   left |mid | right   |
    pub async fn split3(&mut self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) -> (Self, Self) {
        assert!(self.strict_contain(start_vpn, end_vpn));
        // 1. 拆出 “右段”：所有 key >= end
        let right_data_frames = self.data_frames.split_off(&end_vpn);
        //    现在 self.pages 里只剩下 key < end

        // 2. 拆出 “中段”：所有 key >= start（剩下的就是 < start 的“左段”）
        let mid_data_frames = self.data_frames.split_off(&start_vpn);
        //    self.pages 现在只剩下 key < start（就是左段）
        // 3. 准备中段的 backend
        let mid_file = match self.fd.clone() {
            Some(mut mmap_file) => {
                let off = (start_vpn.0 - self.start_vpn().0) << PAGE_SIZE_BITS;
                //目前 mmap_file一定是OsInode
                let file = mmap_file.file.file().unwrap();
                let new_off=file.lseek(off as isize, SEEK_CUR).unwrap();
                file.lseek((new_off-off) as isize, SEEK_SET).unwrap();
                mmap_file.offset=new_off;
                Some(mmap_file)
            }
            None => None,
        };

       

        // 4. 构造 mid 区域
        let mid = MapArea {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: mid_data_frames, 
            map_type: self.map_type,
            map_perm: self.map_perm,
            area_type: self.area_type,
            fd: mid_file,
            mmap_flags: self.mmap_flags,
        };

        // 5. 准备右段的 backend
        let right_file = match self.fd.clone() {
            Some(mut mmap_file) => {
                let off = (end_vpn.0 - self.start_vpn().0) << PAGE_SIZE_BITS;
                //目前 mmap_file一定是OsInode
                let file = mmap_file.file.file().unwrap();
                let new_off=file.lseek(off as isize, SEEK_CUR).unwrap();
                file.lseek((new_off-off) as isize, SEEK_SET).unwrap();
                mmap_file.offset=new_off;
                Some(mmap_file)
            }
            None => None,
        };


        // 6. 构造 right 区域
        let right = MapArea {
            vpn_range: VPNRange::new(end_vpn, self.end_vpn()),
            data_frames: right_data_frames, 
            map_type: self.map_type,
            map_perm: self.map_perm,
            area_type: self.area_type,
            fd: right_file,
            mmap_flags: self.mmap_flags,
        };
        //修改left区域
        self.vpn_range.set_end(start_vpn);
        
        (mid, right)
    }
    ///将一个area拆分为两个
    pub async fn split(&mut self, vpn: VirtPageNum) -> Self {
        let right_data_frames = self.data_frames.split_off(&vpn);
        //  准备mmap_file
        let right_file = match self.fd.clone() {
            Some(mut mmap_file) => {
                let off = (vpn.0 - self.start_vpn().0) << PAGE_SIZE_BITS;
                //目前 mmap_file一定是OsInode
                let file = mmap_file.file.file().unwrap();
                let new_off=file.lseek(off as isize, SEEK_CUR).unwrap();
                file.lseek((new_off-off) as isize, SEEK_SET).unwrap();
                mmap_file.offset=new_off;
                Some(mmap_file)
            }
            None => None,
        };

        // 6. 构造 right 区域
        let right = MapArea {
            vpn_range: VPNRange::new(vpn, self.end_vpn()),
            data_frames: right_data_frames, 
            map_type: self.map_type,
            map_perm: self.map_perm,
            area_type: self.area_type,
            fd: right_file,
            mmap_flags: self.mmap_flags,
        };
        //修改left区域
        self.vpn_range.set_end(vpn);
        right
    }
    // pub fn lazy_page_fault(va: VirtAddr, page_table: &mut PageTable, vma: &mut MapArea) {
    //     // 仅映射页面
    //     vma.map_one(page_table, va.floor()).unwrap();
    //     flush_tlb();
    // }
}

#[derive(Copy, Clone, PartialEq, Debug)]
/// map type for memory set: identical or framed
pub enum MapType {
    Framed,
    Direct,
}

bitflags! {
    /// map permission corresponding to that in pte: `R W X U`
    pub struct MapPermission: u8 {
        ///Readable
        const R = 1 << 1;
        ///Writable
        const W = 1 << 2;
        ///Excutable
        const X = 1 << 3;
        ///Accessible in U mode
        const U = 1 << 4;
    }
}

impl From<MmapProt> for MapPermission {
    fn from(port: MmapProt) -> Self {
        let mut flag: MapPermission = MapPermission::empty();
        flag |= MapPermission::U;
        if port.contains(MmapProt::PROT_READ) {
            flag |= MapPermission::R;
        }
        if port.contains(MmapProt::PROT_WRITE) {
            flag |= MapPermission::W;
        }
        if port.contains(MmapProt::PROT_EXEC) {
            flag |= MapPermission::X;
        }
        flag
    }
}

/// Map area type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapAreaType {
    /// Segments from elf file, e.g. text, rodata, data, bss
    Elf,
    /// Stack
    Stack,
    /// Brk
    Brk,
    /// Mmap
    Mmap,
    /// Shared memory
    Shm,
    /// Physical frames(for kernel)
    Physical,
    /// MMIO(for kernel)
    MMIO,

    Tls,
}
#[derive(Clone)]
pub struct MmapFile {
    pub file: FileDescriptor,
    pub offset: usize,
}

impl MmapFile {
    pub fn new(file: FileDescriptor, offset: usize) -> Self {
        Self { file, offset }
    }

    pub async fn readable(&self) -> TemplateRet<bool> {
        self.file.readable()
    }

    pub async fn writable(&self) -> TemplateRet<bool> {
        self.file.writable()
    }
}
