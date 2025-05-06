//! Implementation of [`MapArea`] and [`MemorySet`].

use crate::config::{ DL_INTERP_OFFSET, KERNEL_DIRECT_OFFSET, KERNEL_PGNUM_OFFSET, USER_HEAP_SIZE};
use crate::fs::{map_dynamic_link_file, open_file, OpenFlags, NONE_MODE};
use crate::task::aux::{Aux, AuxType};
use super::{frame_alloc, FrameTracker};
use super::{PTEFlags, PageTable, PageTableEntry};
use super::{/*PhysAddr,*/ PhysPageNum, VirtAddr, VirtPageNum};
use super::{StepByOne, VPNRange};
use crate::config::{MEMORY_END, MMIO, PAGE_SIZE,/*  TRAMPOLINE, TRAP_CONTEXT_BASE,*/};
use crate::sync::UPSafeCell;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use xmas_elf::ElfFile;
use core::arch::asm;
use lazy_static::*;
use riscv::register::satp;

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss_with_stack();
    fn ebss();
    fn ekernel();
    // fn strampoline();
}

lazy_static! {
    /// The kernel's initial memory mapping(kernel address space)
    pub static ref KERNEL_SPACE: Arc<UPSafeCell<MemorySet>> =
        Arc::new(unsafe { UPSafeCell::new(MemorySet::new_kernel()) });
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
    /// For Trap Context
    Trap,
    /// Shared memory
    Shm,
    /// Physical frames(for kernel)
    Physical,
    /// MMIO(for kernel)
    MMIO,
}
/// the kernel token
pub fn kernel_token() -> usize {
    KERNEL_SPACE.exclusive_access().token()
}

/// address space
pub struct MemorySet {
    ///根页表位置
    pub page_table: PageTable,
    ///memoryset的区域
    pub areas: Vec<MapArea>,
}

impl MemorySet {
  
    ///s
    pub fn unmap_peek(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
    )->bool{

        let var=VPNRange::new(start_va.floor(),end_va.ceil());
        
        for vpn in var{
            let pte=self.page_table.find_pte(vpn);
         match pte{
            Some(p)=>{
                if !p.is_valid(){
                    return true
                }

            }
            None=>{
                return true
            }
         }
        }

        for area in &mut self.areas{
            if area.vpn_range.get_start()==start_va.floor(){
                (*area).unmap(&mut self.page_table);
                break;
            }

        }
        false
    }
    /// 复制逻辑段内容
    pub fn clone_area(&mut self, start_vpn: VirtPageNum, another: &MemorySet) {
        if let Some(area) = another
            .areas
            .iter()
            .find(|area| area.vpn_range.get_start() == start_vpn)
        {
            for vpn in area.vpn_range {
                let src_ppn = another.translate(vpn).unwrap().ppn();
                let dst_ppn = self.translate(vpn).unwrap().ppn();
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
            }
        }
    }
    ///s
    pub fn insert_framed_area_peek_for_mmap(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    )->bool{

        let var=VPNRange::new(start_va.floor(),end_va.ceil());
        
        for vpn in var{
            let pte=self.page_table.find_pte(vpn);
         
                if let Some(a)= pte{
                    if  a.is_valid() {
                        return true;
                    }
               
                
            }
        }
        self.insert_framed_area(start_va, end_va, permission, MapAreaType::Mmap);
        false
    }
///Create a new `MemorySet` from global kernel space
    pub fn new_from_kernel()->Self{
        let page_table = PageTable::new_from_kernel();

        let areas= Vec::new();
        Self { page_table, areas }
    }
    /// Create a new empty `MemorySet`.
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }
    /// Get the page table token
    pub fn token(&self) -> usize {
        self.page_table.token()
    }
    /// Assume that no conflicts.
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
        area_type: MapAreaType,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission, area_type),
            None,

        );
    }
    /// remove a area
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some((idx, area)) = self
            .areas
            .iter_mut()
            .enumerate()
            .find(|(_, area)| area.vpn_range.get_start() == start_vpn)
        {
            area.unmap(&mut self.page_table);
            self.areas.remove(idx);
        }
    }

    ///有页内偏移的push
    fn push_with_offset(&mut self, mut map_area: MapArea, offset: usize, data: Option<&[u8]>)  {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data, offset);
        }
        self.areas.push(map_area);
    }
    /// Add a new MapArea into this MemorySet.
    /// Assuming that there are no conflicts in the virtual address
    /// space.
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data,0);
        }
        self.areas.push(map_area);
    }
    ///通过递归调整提示地址，从高到低寻找足够大的未占用虚拟地址空间，确保新分配区域不与现有区域重叠。
    pub fn find_insert_addr(&self, hint: usize, size: usize) -> usize {
        let end_vpn = VirtAddr::from(hint).floor();
        let start_vpn = VirtAddr::from(hint - size).floor();
        for area in self.areas.iter() {
            let (start, end) = area.vpn_range.range();
            if end_vpn > start && start_vpn < end {
                let new_hint = VirtAddr::from(start_vpn).0 - PAGE_SIZE;
                return self.find_insert_addr(new_hint, size);
            }
        }
        VirtAddr::from(start_vpn).0
    }
    /// Mention that trampoline is not collected by areas.
    // fn map_trampoline(&mut self) {
    //     self.page_table.map(
    //         VirtAddr::from(TRAMPOLINE).into(),
    //         PhysAddr::from(strampoline as usize).into(),
    //         PTEFlags::R | PTEFlags::X,
    //     );
    // }
    /// Without kernel stacks.
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();
        // map trampoline
        // memory_set.map_trampoline();
        // map kernel sections
        info!("kernel  token: {:#x}",memory_set.token());
        info!(".text [{:#x}, {:#x})", stext as usize, etext as usize);
        info!(".rodata [{:#x}, {:#x})", srodata as usize, erodata as usize);
        info!(".data [{:#x}, {:#x})", sdata as usize, edata as usize);
        info!(
            ".bss [{:#x}, {:#x})",
            sbss_with_stack as usize, ebss as usize
        );
        info!("mapping .text section");
        memory_set.push(
            MapArea::new(
                (stext as usize).into(),
                (etext as usize).into(),
                MapType::Direct,
                MapPermission::R | MapPermission::X,
                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping .rodata section");
        memory_set.push(
            MapArea::new(
                (srodata as usize).into(),
                (erodata as usize).into(),
                MapType::Direct,
                MapPermission::R,

                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping .data section");
        memory_set.push(
            MapArea::new(
                (sdata as usize).into(),
                (edata as usize).into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                
                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping .bss section");
        memory_set.push(
            MapArea::new(
                (sbss_with_stack as usize).into(),
                (ebss as usize).into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping physical memory");
        memory_set.push(
            MapArea::new(
                (ekernel as usize).into(),
                MEMORY_END.into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                MapAreaType::Physical,
            ),
            None,
        );
        info!("mapping memory-mapped registers");
      
        for pair in MMIO {
  debug!("MMio:{:#x},{:#x}",(*pair).0+KERNEL_DIRECT_OFFSET,(*pair).1+(*pair).0+KERNEL_DIRECT_OFFSET   );
            memory_set.push(
                MapArea::new(
                    ((*pair).0+KERNEL_DIRECT_OFFSET).into(),
                    ((*pair).0 + (*pair).1+KERNEL_DIRECT_OFFSET).into(),
                    MapType::Direct,
                    MapPermission::R | MapPermission::W,
                    MapAreaType::MMIO,
                ),
                None,
            );
        }
        memory_set
    }
    fn map_elf(&mut self, elf: &ElfFile, offset: VirtAddr) -> (VirtPageNum, VirtAddr) {
        let elf_header = elf.header;
        let ph_count = elf_header.pt2.ph_count();

        let mut max_end_vpn = offset.floor();
        let mut header_va = 0;
        let mut has_found_header_va = false;

        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == xmas_elf::program::Type::Load {
                let start_va: VirtAddr = (ph.virtual_addr() as usize + offset.0).into();
                let end_va: VirtAddr =
                    ((ph.virtual_addr() + ph.mem_size()) as usize + offset.0).into();
                if !has_found_header_va {
                    header_va = start_va.0;
                    has_found_header_va = true;
                }
                let mut map_perm = MapPermission::U;
                let ph_flags = ph.flags();
                if ph_flags.is_read() {
                    map_perm |= MapPermission::R;
                }
                if ph_flags.is_write() {
                    map_perm |= MapPermission::W;
                }
                if ph_flags.is_execute() {
                    map_perm |= MapPermission::X;
                }
                let map_area = MapArea::new(
                    start_va,
                    end_va,
                    MapType::Framed,
                    map_perm,
                    MapAreaType::Elf,
                );
                let data_offset = start_va.0 - start_va.floor().0 * PAGE_SIZE;
                max_end_vpn = map_area.vpn_range.get_end();
                self.push_with_offset(
                    map_area,
                    data_offset,
                    Some(&elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize]),
                );
            }
        }
        (max_end_vpn, header_va.into())
    }
    /// Include sections in elf and trampoline and TrapContext and user stack,
    /// also returns user_sp_base and entry point.
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize,Vec<Aux>) {
        let mut auxv = Vec::new();
        let mut memory_set = Self::new_from_kernel();
        // map trampoline
        // memory_set.map_trampoline();
        // map program headers of elf, with U flag
        
        let elf = xmas_elf::ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let ph_count = elf_header.pt2.ph_count();
        let mut entry_point = elf_header.pt2.entry_point() as usize;
        auxv.push(Aux::new(
            AuxType::PHENT,
            elf.header.pt2.ph_entry_size() as usize,
        )); // ELF64 header 64bytes
        auxv.push(Aux::new(AuxType::PHNUM, ph_count as usize));
        auxv.push(Aux::new(AuxType::PAGESZ, PAGE_SIZE as usize));
        let mut is_dl = false;
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == xmas_elf::program::Type::Interp {
                is_dl = true;
                break;
            }
        }
        // 设置动态链接
        if is_dl  {

            debug!("[load_dl] encounter a dl elf");
            let section = elf.find_section_by_name(".interp").unwrap();
            let mut interp = String::from_utf8(section.raw_data(&elf).to_vec()).unwrap();
            interp = interp.strip_suffix("\0").unwrap_or(&interp).to_string();
            debug!("[load_dl] interp {}", interp);

            let interp = map_dynamic_link_file(&interp);

            // log::info!("interp {}", interp);

            let interp_inode = open_file(&interp, OpenFlags::O_RDONLY, NONE_MODE)
                .unwrap()
                .file()
                .ok();
            let interp_file = interp_inode.unwrap();
            let interp_elf_data = interp_file.read_all();
            let interp_elf = xmas_elf::ElfFile::new(&interp_elf_data).unwrap();
            memory_set.map_elf(&interp_elf, DL_INTERP_OFFSET.into());

            let interp_entry_point = interp_elf.header.pt2.entry_point() as usize + DL_INTERP_OFFSET;



            auxv.push(Aux::new(AuxType::BASE, DL_INTERP_OFFSET));
            entry_point = interp_entry_point;
        } else {
            auxv.push(Aux::new(AuxType::BASE, 0));
        }
        auxv.push(Aux::new(AuxType::FLAGS, 0 as usize));
        auxv.push(Aux::new(
            AuxType::ENTRY,
            elf.header.pt2.entry_point() as usize,
        ));
        auxv.push(Aux::new(AuxType::UID, 0 as usize));
        auxv.push(Aux::new(AuxType::EUID, 0 as usize));
        auxv.push(Aux::new(AuxType::GID, 0 as usize));
        auxv.push(Aux::new(AuxType::EGID, 0 as usize));
        auxv.push(Aux::new(AuxType::PLATFORM, 0 as usize));
        auxv.push(Aux::new(AuxType::HWCAP, 0 as usize));
        auxv.push(Aux::new(AuxType::CLKTCK, 100 as usize));
        auxv.push(Aux::new(AuxType::SECURE, 0 as usize));
        auxv.push(Aux::new(AuxType::NOTELF, 0x112d as usize));
        let (max_end_vpn, head_va) = memory_set.map_elf(&elf, VirtAddr(0));
         // Get ph_head addr for auxv
         let ph_head_addr = head_va.0 + elf.header.pt2.ph_offset() as usize;
         auxv.push(Aux {
             aux_type: AuxType::PHDR,
             value: ph_head_addr as usize,
         });
        // map user stack with U flags  
        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut user_heap_bottom: usize = max_end_va.into();
        // guard page
        user_heap_bottom += PAGE_SIZE;
        let user_heap_top = user_heap_bottom;

    // used in sbrk
        memory_set.push(
            MapArea::new(
                user_heap_bottom.into(),
                user_heap_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
                MapAreaType::Brk,
            ),
            None,
        );
//  println!("sbrk ppn :");
        // for (_,frame) in memory_set.areas.last().unwrap().data_frames.iter(){
        //     println!("ppn:{:#x}",frame.ppn().0);
        // }
       
       
    

                trace!("user_heap_sp,start_va::{:#x}, sp,end_va::{:#x}",user_heap_bottom,user_heap_top);
                trace!("app_entry:{:#x}",entry_point);
 

       
        (
            memory_set,
            user_heap_bottom,
           entry_point,
           auxv,
        )
    }
    /// Create a new address space by copy code&data from a exited process's address space.
    pub fn from_existed_user(user_space: &Self) -> Self {
        let mut memory_set = Self::new_from_kernel();
        // map trampoline
        // memory_set.map_trampoline();
        // copy data sections/trap_context/user_stack
        for area in user_space.areas.iter().filter(|a| a.area_type != MapAreaType::Stack&&a.area_type != MapAreaType::Trap) {
            let new_area = MapArea::from_another(area);
            memory_set.push(new_area, None);
        
            // 复制数据到新空间
            for vpn in area.vpn_range.clone() {
                let src_ppn = user_space.translate(vpn).unwrap().ppn();
                let dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
            }
        }
        
        memory_set
    }
    /// Change page table by writing satp CSR Register.
    pub fn activate(&self) {
        let satp = self.page_table.token();
        // trace!("activate new page table token:{:#x}",satp);
        unsafe {
            satp::write(satp);
            asm!("sfence.vma");
        }
        // 
        // trace!("activated");
    }
    /// Translate a virtual page number to a page table entry
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {

        self.page_table.translate(vpn)
    }

    ///Remove all `MapArea`
    pub fn recycle_data_pages(&mut self) {
        self.page_table.clear();
        self.areas.clear();
    }

    /// shrink the area to new_end
    #[allow(unused)]
    pub fn shrink_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.shrink_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    /// append the area to new_end
    #[allow(unused)]
    pub fn append_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.append_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }
}
/// map area structure, controls a contiguous piece of virtual memory
pub struct MapArea {
    ///从start到end的vpn
    pub vpn_range: VPNRange,
    pub data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    map_type: MapType,
    map_perm: MapPermission,
    area_type: MapAreaType,
    
}

impl MapArea {
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
        }
    }
    pub fn from_another(another: &Self) -> Self {
        Self {
            vpn_range: VPNRange::new(another.vpn_range.get_start(), another.vpn_range.get_end()),
            data_frames: BTreeMap::new(),
            map_type: another.map_type,
            map_perm: another.map_perm,
            area_type: another.area_type,
        }
    }
    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let ppn: PhysPageNum;
      
        match self.map_type {
           
            MapType::Framed => {
                let frame = frame_alloc().unwrap();
                ppn = frame.ppn();
               
                self.data_frames.insert(vpn, frame);
            }
            MapType::Direct => {
                ppn = PhysPageNum(vpn.0 - KERNEL_PGNUM_OFFSET);
            }
        }
        let pte_flags = PTEFlags::from_bits(self.map_perm.bits).unwrap();
        page_table.map(vpn, ppn, pte_flags);
    }
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if self.map_type == MapType::Framed {
            self.data_frames.remove(&vpn);
        }
        page_table.unmap(vpn);
    }
    pub fn map(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.map_one(page_table, vpn);
        }
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
    pub fn append_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(self.vpn_range.get_end(), new_end) {
            self.map_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }
    /// data: start-aligned but maybe with shorter length
    /// assume that all frames were cleared before
    pub fn copy_data(&mut self, page_table: &mut PageTable, data: &[u8], offset: usize) {
        assert_eq!(self.map_type, MapType::Framed);
        assert!(offset<PAGE_SIZE);
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
}

#[derive(Copy, Clone, PartialEq, Debug)]
/// map type for memory set: identical or framed
pub enum MapType {
    Framed,
    Direct  ,
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

/// remap test in kernel space
#[allow(unused)]
pub fn remap_test() {
    
    info!("[kernel]:remap testing");
    let mut kernel_space = KERNEL_SPACE.exclusive_access();
    let mid_text: VirtAddr = (stext as usize + (etext as usize - stext as usize) / 2).into();
    let mid_rodata: VirtAddr =
        (srodata as usize + (erodata as usize - srodata as usize) / 2).into();
    let mid_data: VirtAddr = (sdata as usize + (edata as usize - sdata as usize) / 2).into();
    assert!(!kernel_space
        .page_table
        .translate(mid_text.floor())
        .unwrap()
        .writable(),);
    debug!("text pass");
    assert!(!kernel_space
        .page_table
        .translate(mid_rodata.floor())
        .unwrap()
        .writable(),);
    debug!("rodata pass");
    assert!(!kernel_space
        .page_table
        .translate(mid_data.floor())
        .unwrap()
        .executable(),);
    println!("remap_test passed!");
}

