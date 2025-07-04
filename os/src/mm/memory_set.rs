//! Implementation of [`MapArea`] and [`MemorySet`].
use crate::config::{ DL_INTERP_OFFSET, KERNEL_DIRECT_OFFSET, MMAP_PGNUM_TOP, PAGE_SIZE_BITS};
use crate::fs::{map_dynamic_link_file, open_file, File, OpenFlags, NONE_MODE};
use crate::mm::shm::SHM_MANAGER;
use crate::mm::{ area, translated_byte_buffer, FrameTracker, UserBuffer, VPNRange, KERNEL_PAGE_TABLE_TOKEN};
use crate::syscall::flags::MremapFlags;
use crate::task::auxv::{Aux, AuxType};
use crate::task::current_process;
use crate::utils::error::{GeneralRet, SysErrNo, SyscallRet, TemplateRet};
use super::area::{MapArea, MapAreaType, MapPermission, MapType, VmAreaTree};
use super::page_table::{ PutDataError, PutDataRet};
use super::{flush_all, KernelAddr, MmapFlags, PhysAddr, StepByOne, TranslateError, VirtAddr, VirtPageNum};
use super::{PageTable, PageTableEntry};
use crate::config::{MEMORY_END, MMIO, PAGE_SIZE,/*  TRAMPOLINE, TRAP_CONTEXT_BASE,*/};
use alloc::collections::btree_map::{BTreeMap};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_SET};
use xmas_elf::ElfFile;
use core::arch::asm;
use core::ops::Range;
use core::{ptr, slice};
#[cfg(target_arch = "riscv64")]
use riscv::register::satp;

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss_with_stack();
    fn _ebss();
    fn _ekernel();
     fn strampoline();

     fn etrampoline();
}

   

/// address space
pub struct MemorySet {
    ///根页表位置
    pub page_table: PageTable,
    ///memoryset的区域
    pub areatree: VmAreaTree,
}

// 新建 PageFaultError 枚举，把原来所有 `return false` 的情况都列出来
#[derive(Debug)]
pub enum PageFaultError {
    /// 找不到包含 fault_va 的映射区
    AreaNotFound,
    /// 找到的映射区类型不是 Mmap
    NotMmapType,
    /// 映射区的 vpn_range 为空，或者该页已经分配
    RangeEmpty,
    AlreadyAllocated,
    /// 虚拟页号虽在 area.vpn_range 内，但不满足懒分配条件
    VpnNotHandled,
    __,
}
impl From<PageFaultError> for SysErrNo {
    fn from(err: PageFaultError) -> SysErrNo {
        match err {
            PageFaultError::AreaNotFound => SysErrNo::EFAULT,
            PageFaultError::NotMmapType => SysErrNo::EINVAL,
            PageFaultError::RangeEmpty => SysErrNo::EBUSY,
            PageFaultError::VpnNotHandled => SysErrNo::EINVAL,
            PageFaultError::AlreadyAllocated => SysErrNo::EEXIST,
            PageFaultError::__ => SysErrNo::EFAULT,
        }
    }
}

impl MemorySet {

   /// unmap 拆分 这个range两端的area
pub fn munmap(
    &mut self,
    new_start: VirtPageNum,
    new_end: VirtPageNum,
   
)  {
    // 1. 找到所有与 [new_start, new_end) 有交集的旧 MapArea
    let mut overlaps = Vec::new();
    for (&start, area) in self.areatree.range(..new_end) {
        if area.end_vpn().0 > new_start.0 {
            overlaps.push(start);
        }
    }

    // 2. 对每个重叠的 area 进行拆分并保留左右两段
    for start in overlaps {
        let  area = self.areatree.remove(&start).unwrap();
        let a0 = area.start_vpn();
        let a1 = area.end_vpn();
        let r0 = new_start;
        let r1 = new_end;

        // 左段：如果有
        if a0 < r0 {
            let  left = area.from_another_with_range(a0, r0);
            self.areatree.push(left);
        }
        // 右段：如果有
        if a1 > r1 {
            let  right = area.from_another_with_range(r1, a1);
            self.areatree.push(right);
        }

        // **中间这部分 [r0, r1) 是要给新 area 用的**，
        // 先把对应的 PTE 一个个清掉
    
        for vpn in r0.0.. r1.0 {
            
        if area.allocated(VirtPageNum(vpn))

           { self.page_table.unmap(VirtPageNum(vpn));}
        }
    
        flush_all();
    }

   
 }
    
    /// 复制逻辑段内容
    pub fn clone_area(&mut self, start_vpn: VirtPageNum, another:&MemorySet) {
    //    self.areatree.debug_print();
       trace!("b");
    //    another.areatree.debug_print();

        if let Some(area) = another
            .areatree.get(&start_vpn)
            
        {
            
            for vpn in area.vpn_range {
                let src_ppn = another.translate(vpn).expect(&format!("translate failed in `another` for vpn {:#x}", vpn.0)).ppn();
                let dst_ppn = self.translate(vpn).expect(&format!("translate failed in `self` for vpn {:#x}", vpn.0)).ppn();
                dst_ppn
                    .get_bytes_array()
                    .copy_from_slice(src_ppn.get_bytes_array());
            }
        }
    }
    // 从 `src_space` 的 `src_area` 复制数据到当前空间的 `dst_area`。
    /// 要求两者的 vpn_range.size() 相同，否则 panic。
    pub fn copy_area_data(
        &mut self,
        src_space: &MemorySet,
        src_vpn: VirtPageNum,
        dst_vpn: VirtPageNum,
    ) {
        // self.areatree.debug_print();
        // 1. 断言大小一致
        let src_area = src_space
            .areatree
            .get(&src_vpn)
            .expect(&format!("src_space missing area for vpn {:#x}", src_vpn.0));

        let dst_area = self
            .areatree
            .get_mut(&dst_vpn)
            .expect(&format!("dst_space missing area for vpn {:#x}", dst_vpn.0));
        let src_pages = src_area.range_size();
        let dst_pages = dst_area.range_size();
        assert!(
            src_pages == dst_pages,
            "copy_area_data: src_pages ({}) != dst_pages ({})",
            src_pages,
            dst_pages
        );
        
           

            // Insert mapping with shared frames
            dst_area.map_given_frames(&mut self.page_table, &src_area.data_frames,true);
               
    }
   
///Create a new `MemorySet` from global kernel space
    pub fn new_from_kernel()->Self{
        let page_table = PageTable::new_from_kernel();

        let areas=VmAreaTree ::new();
        Self { page_table, areatree: areas }
    }
    /// Create a new empty `MemorySet`.
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areatree: VmAreaTree::new(),
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
    ) ->GeneralRet{
      
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission, area_type),
            None,

        )
    }
    /// remove a area
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
       
        if let Some((_, mut area)) = self.areatree.remove_entry(&start_vpn) {
            area.unmap(&mut self.page_table);
        }
    }

    ///有页内偏移的push
    fn push_with_offset(&mut self, mut map_area: MapArea, offset: usize, data: Option<&[u8]>) ->GeneralRet {
        map_area.map(&mut self.page_table)?;
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data, offset);
        }
        self.areatree.push(map_area);
        Ok(())
    }
    /// Add a new MapArea into this MemorySet.
    /// Assuming that there are no conflicts in the virtual address
    /// space.
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>)->GeneralRet{

        map_area.map(&mut self.page_table)?;
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data,0);
        }

       
        self.areatree.push(map_area);
        Ok(())
    }

#[cfg(target_arch = "loongarch64")]
pub fn new_kernel() ->Self{
        // map trampoline
        // memory_set.map_trampoline();
        // map kernel sections
        info!(".text [{:#x}, {:#x})", stext as usize, etext as usize);
        info!(".rodata [{:#x}, {:#x})", srodata as usize, erodata as usize);
        info!(".data [{:#x}, {:#x})", sdata as usize, edata as usize);
        info!(
            ".bss [{:#x}, {:#x})",
            sbss_with_stack as usize, _ebss as usize
        );
        Self::new_bare()
}
   
#[cfg(target_arch = "riscv64")]
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
            sbss_with_stack as usize, _ebss as usize
        );
info!("mapping .text[stext..stextsig] (kernel only) range:{:#x}-{:#x}",stext as usize,strampoline as usize);
let _ = memory_set.push(
    MapArea::new(
        (stext as usize).into(),
        (strampoline as usize).into(),
        MapType::Direct,
        MapPermission::R | MapPermission::X,   // kernel only
        MapAreaType::Elf,
    ),
    None,
);


info!("mapping .text.signal_trampoline[stextsig..etextsig] (user-exec),range:{:#x}-{:#x}",strampoline as usize,etrampoline as usize);
let _ = memory_set.push(
    MapArea::new(
        (strampoline as usize).into(),
        (etrampoline as usize).into(),
        MapType::Direct,
        MapPermission::R | MapPermission::X | MapPermission::U, // allow user to execute
        MapAreaType::Elf,  // 或者专门定义一个 MapAreaType::SignalTrampoline
    ),
    None,
);
info!("mapping .text[etextsig..etext] (kernel only),range:{:#x}-{:#x}",etrampoline as usize,etext as usize);
let _ = memory_set.push(
    MapArea::new(
        (etrampoline as usize).into(),
        (etext as usize).into(),
        MapType::Direct,
        MapPermission::R | MapPermission::X,   // kernel only
        MapAreaType::Elf,
    ),
    None,
);

        info!("mapping .rodata section:{:#x},{:#x}",
        (srodata as usize),
        (erodata as usize),);
        let _ = memory_set.push(
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
        let _ = memory_set.push(
            MapArea::new(
                (sdata as usize).into(),
                (edata as usize).into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                
                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping .bss section {:#x},{:#x}" , (sbss_with_stack as usize),(_ebss as usize) );
        let _ = memory_set.push(
            MapArea::new(
                (sbss_with_stack as usize).into(),
                (_ebss as usize).into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                MapAreaType::Elf,
            ),
            None,
        );
        info!("mapping physical memory {:#x} -{:#x}  ",(_ekernel as usize),MEMORY_END);
        let _ = memory_set.push(
            MapArea::new(
                (_ekernel as usize).into(),
                MEMORY_END.into(),
                MapType::Direct,
                MapPermission::R | MapPermission::W,
                MapAreaType::Physical,
            ),
            None,
        );
      
        info!("mapping memory-mapped registers");
        
         for pair in MMIO {
  info!("MMio:{:#x},{:#x}",(*pair).0+KERNEL_DIRECT_OFFSET,(*pair).1+(*pair).0+KERNEL_DIRECT_OFFSET   );
            let _ = memory_set.push(
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
    fn map_elf(&mut self, elf: &ElfFile, offset: VirtAddr) -> TemplateRet<(VirtPageNum, VirtAddr,usize)> {
        let tls=0;
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
log::info!("[map_elf_load] segment {}: file_offset=0x{:x}, mem_size=0x{:x}, start_va=0x{:x}, end_va=0x{:x}",
                i,
                ph.offset(),
                ph.mem_size(),
                start_va.0,
                end_va.0,
               
            );
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
                if ph_flags.is_execute() || ph.get_type() == Ok(xmas_elf::program::Type::GnuRelro) {
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
                )?;
            } 
            else  if ph.get_type().unwrap()== xmas_elf::program::Type::Tls  {

                
            
                       }
        }
        Ok((max_end_vpn, header_va.into(),tls))
    }
    /// Include sections in elf and trampoline and TrapContext and user stack,
    /// also returns user_sp_base and entry point.
    /// 
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize,Vec<Aux>,bool) {
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
       let base= if is_dl  {

            debug!("[load_dl] encounter a dl elf");
            let section = elf.find_section_by_name(".interp").unwrap();
            let mut interp = String::from_utf8(section.raw_data(&elf).to_vec()).unwrap();
            interp = interp.strip_suffix("\0").unwrap_or(&interp).to_string();
            debug!("[load_dl] interp {}", interp);

            let interp = map_dynamic_link_file(&interp);

            // log::info!("interp {}", interp);

            let interp_inode = open_file(&interp, OpenFlags::O_RDONLY, NONE_MODE)
                .expect(&format!("can't find dl path :{}",interp))
                .file()
                .ok();
            let interp_file = interp_inode.unwrap();
            let interp_elf_data = interp_file.read_all();
            let interp_elf = xmas_elf::ElfFile::new(&interp_elf_data).unwrap();
            memory_set.map_elf(&interp_elf, DL_INTERP_OFFSET.into()).unwrap();

            let interp_entry_point = interp_elf.header.pt2.entry_point() as usize + DL_INTERP_OFFSET;



            auxv.push(Aux::new(AuxType::BASE, DL_INTERP_OFFSET));
            entry_point = interp_entry_point;
            DL_INTERP_OFFSET
        } else {
            trace!("no dl");
            auxv.push(Aux::new(AuxType::BASE, 0));
            0
        };
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
  
        let (max_end_vpn, head_va,_tls) = memory_set.map_elf(&elf, VirtAddr(0)).unwrap();
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
        ).unwrap();
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
            is_dl,
        )
    }
   
    pub fn push_with_given_frames(&mut self, mut map_area: MapArea, frames: &BTreeMap<VirtPageNum,Arc<FrameTracker>>,is_cow:bool) {
        map_area.map_given_frames(&mut self.page_table, frames,is_cow);

    //    println!("start_vpn:{:#x},flags:{:?}",map_area.start_vpn().0, self.page_table.translate(map_area.start_vpn()).unwrap().flags());
        self.areatree.push(map_area);
           
    }

    /// Create a new address space by copying code & data from an existing process's address space,
/// using Copy-On-Write for private mappings and sharing for shared mappings.
pub async fn from_existed_user(user_space: &mut Self) -> Self {
    let mut memory_set = Self::new_from_kernel();

    // Only process each area once
    {
        let old_areatree = &mut user_space.areatree;
        let old_page_table_ref = &user_space.page_table;
        for (_, area) in old_areatree.iter_mut().filter(|(_,f)|f.area_type != MapAreaType::Stack) {
            if area.area_type == MapAreaType::Mmap && area.mmap_flags.contains(MmapFlags::MAP_SHARED) {
                // Shared mapping: reuse original frames
               
                let new_area = MapArea::from_another(area);
                memory_set.push_with_given_frames(new_area, &area.data_frames,false);
            } else if area.area_type == MapAreaType::Mmap 
          
            
            
            {
                // Private or other mappings: use COW
                // Clone frame trackers to bump reference counts
                let mut new_area = MapArea::from_another(area);
                new_area.mmap_flags.insert(MmapFlags::MAP_PRIVATE);

                // Insert mapping with shared frames
                memory_set.push_with_given_frames(new_area, &area.data_frames,true);
              // 对每对 (vpn, frame) 做映射并记录
        for (vpn, _) in area.data_frames.iter(){
              let   pte= user_space.page_table.find_pte(*vpn).unwrap();
                pte.set_cow();
            }
            }
            else if let MapAreaType::Shm { shmid } = area.area_type {
              let guard= SHM_MANAGER.lock().await;
              let shm= guard.id_to_segment.get(&shmid).unwrap();
              shm.lock().await.attach(current_process().get_pid() as u32);
              let new_area = MapArea::from_another(area);
              memory_set.push_with_given_frames(new_area,
                  &area.data_frames,
                  /* is_cow = */ false
              );

            }

           
            else{


                let area_to_push = MapArea::from_another(area);
                memory_set.push(area_to_push, None).unwrap();
                for vpn in area.vpn_range {
                    let src_ppn = old_page_table_ref.translate(vpn).unwrap().ppn();
                    let dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                    // println!("copying vpn:{:#x} src_ppn:{:#x} dst_ppn:{:#x}",vpn.0,src_ppn.0,dst_ppn.0);
                    dst_ppn
                        .get_bytes_array()
                        .copy_from_slice(src_ppn.get_bytes_array());
                } 
            }



        }
    }

    memory_set
}
pub async  fn from_existed_user1(user_space: &mut Self) -> Self {
    let mut memory_set = Self::new_from_kernel();
    
    // 第一阶段：收集需要分配的VPN
    let mut vpns_to_alloc = Vec::new();
    
    {
        // 仅借用 areatree
        let old_areatree = &mut user_space.areatree;
        for (_, area) in old_areatree.iter_mut() {
            if area.area_type == MapAreaType::Mmap && 
               !area.mmap_flags.contains(MmapFlags::MAP_SHARED) 
            {
                for vpn in area.vpn_range {
                    if !area.allocated(vpn) {
                        vpns_to_alloc.push(vpn);
                    }
                }
            }
        }
    } // 结束 areatree 的借用
    
    // 第二阶段：处理缺页
    for vpn in vpns_to_alloc {
        user_space.handle_page_fault(vpn.0 << PAGE_SIZE_BITS,true).await.expect(&format!("Failed to handle page fault for VPN {:#x}", vpn.0));
    }
    
    // 第三阶段：处理所有区域
    {
        let old_areatree = &mut user_space.areatree;
        // old_areatree.debug_print();
        let old_page_table = &mut user_space.page_table;
        
        for (_, area) in old_areatree.iter_mut() {
            let new_area = MapArea::from_another(area);
            
            if area.area_type == MapAreaType::Mmap && 
               area.mmap_flags.contains(MmapFlags::MAP_SHARED) 
            {
                
                memory_set.push_with_given_frames(new_area, &area.data_frames.clone(),false);
            } else {
                // 其他区域
                // new_area.debug_print();
                memory_set.push(new_area, None).unwrap();
                
                // 复制数据
                for vpn in area.vpn_range {
                    let src_ppn = old_page_table.translate(vpn).unwrap().ppn();
                    let dst_ppn = memory_set.page_table.translate(vpn).unwrap().ppn();
                    dst_ppn
                        .get_bytes_array()
                        .copy_from_slice(src_ppn.get_bytes_array());
                }
            }
        }
    }
   
    memory_set
}
    /// Change page table by writing satp CSR Register.
    pub fn activate(&self) {
        #[cfg(target_arch = "riscv64")]
        unsafe {
            let satp = self.page_table.token();
            satp::write(satp);
            asm!("sfence.vma");
        }
        #[cfg(target_arch = "loongarch64")]
        unsafe {
            // LoongArch64 的页表基址寄存器是 PGDL 和 PGDH
            use loongArch64::register::pgdl;
            let mut satp = self.page_table.token();
            satp=satp<<PAGE_SIZE_BITS;
            pgdl::set_base(satp);
            asm!("invtlb 0x0, $zero, $zero"); // 刷新所有 TLB 项
        }
    }
   
   pub async  fn safe_translate(&mut self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        // Check if the page table is valid
        
        let va = VirtAddr::from(vpn).0;
        loop{
        match self.page_table.translate(vpn) {
            None => {
               if self.handle_page_fault(va,true).await.is_ok(){
                    // If the page fault is handled successfully, retry translation
                     
                    continue;
                } else {
                    // If the page fault cannot be handled, return None
                    break None
               }
            }
            Some(ref pte) => {
                if !pte.is_valid() {
                    //这里不确定有没有问题todo
                   if self.handle_page_fault(va,true).await.is_ok(){
                    // If the page fault is handled successfully, retry translation
                    continue;
                   }
                   else{
                    // If the page fault cannot be handled, return None
                    break None
                   };
                }
                else{
                    break Some(*pte);
                }
            }
    }   
    
  }
}
    ///Remove all `MapArea`
    pub async  fn recycle_data_pages(&mut self) -> SyscallRet {
        // 先检测是否需要munmap
        for (_,area) in self.areatree.iter_mut() {
            if area.area_type == MapAreaType::Mmap {
                if area.mmap_flags.contains(MmapFlags::MAP_SHARED)
                    && area.map_perm.contains(MapPermission::W)
                {
                    let addr: VirtAddr = area.vpn_range.get_start().into();
                    let mapped_len: usize = area
                        .vpn_range
                        .into_iter()
                        .filter(|vpn| area.data_frames.contains_key(&vpn))
                        .count()
                        * PAGE_SIZE;
                    let file = area.fd.clone().unwrap();
                    file.file.write(UserBuffer {
                        buffers: translated_byte_buffer(
                            self.page_table.token(),
                            addr.0 as *const u8,
                            mapped_len,
                        )
                        ,
                    }).await?;
                }
            }
        }
        self.areatree.clear();
        self.page_table.clear();
        Ok(0)
    }
pub async  fn safe_translate_va(&mut self, va: VirtAddr) -> Option<PhysAddr> {
    self.safe_translate(va.clone().floor()).await.map(|pte| {
        let aligned_pa: PhysAddr = pte.ppn().into();
        let offset = va.page_offset();
        let aligned_pa_usize: usize = aligned_pa.into();
        (aligned_pa_usize + offset).into()
    })
}
     
    /// Translate a virtual page number to a page table entry
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {

        self.page_table.translate(vpn)
    }

     /// shrink the area to new_end
    #[allow(unused)]
    pub fn shrink_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areatree.get_mut(&start.floor())
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
        .areatree.get_mut(&start.floor())
        {
            area.append_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }
    pub async  fn manual_alloc_range_for_lazy(&mut self, start_va: VirtAddr, end_va: VirtAddr)->Result<(), PageFaultError>{
        assert!(start_va <= end_va);
        
        let mut vpn = start_va.floor();
        let end=end_va.ceil();
     
        while vpn < end {
           match self.handle_page_fault(VirtAddr::from(vpn).0 ,true).await{
            
            Err(e) => 
{
    // trace!("[ manual_alloc_range_for_lazy] err:{:#?},vpn:{:#x}",e,vpn.0);
                if matches!(e, PageFaultError::NotMmapType | PageFaultError::RangeEmpty| PageFaultError::AlreadyAllocated) {
                    {
                  

                    }
                } else {
                    warn!("[manual_alloc_range_for_lazy]err:{:#?}",e);
                    return Err(e);
                }
            },
            Ok(_)=>{},
                   };
            vpn.step(); 
        }
        Ok(())
    }
    pub fn is_region_alloc(&self, start_va: VirtAddr, end_va: VirtAddr) -> bool {
        
        self.areatree.is_fully_contained(&core::ops::Range {
            start: start_va.floor(),
            end: end_va.ceil(),
        })
    

    }

    pub  async  fn safe_put_data<T:Copy +'static>(&mut self, ptr: *mut T, data: T) -> PutDataRet {
        let data_size = core::mem::size_of::<T>();
        if data_size == 0 {
            return Ok(()); // 零大小类型无需写入
        }
         
        let start_va = VirtAddr::from(ptr as usize);
        self.manual_alloc_range_for_lazy(start_va, (start_va.0+data_size).into()).await
        .map_err(|_|PutDataError::TranslationFailed(start_va))?;

        let page_table = &self.page_table;
        // 尝试翻译起始虚拟地址
        let start_pa = page_table
            .translate_va(start_va)
            .ok_or(PutDataError::TranslationFailed(start_va))?;
    
      
        let crosses_page_boundary = if data_size > 0 { // 避免 data_size - 1 溢出
            // 判断起始物理页号和结束物理页号是否不同
            // (pa.as_usize() & !(PAGE_SIZE - 1)) != ((pa.as_usize() + size - 1) & !(PAGE_SIZE - 1))
            start_pa.floor() != (start_pa + (data_size - 1)).floor()
        } else {
            false // 零大小类型不会跨页
        };
    
        if !crosses_page_boundary {
            // 数据完全在单页内
            // 如果对齐允许，可以尝试直接写入
         
            // 若 `T` 对齐要求较高且 `ptr` 可能不对齐，`ptr.write_unaligned(data)` 更安全
            // 安全：调用者保证 ptr 对 T 来说有效且可写
            unsafe { ptr::write(start_pa.get_mut(), data) };
            
            // 或者：
            // let target_mut_ref: &mut T = &mut *start_pa.as_mut_ptr::<T>();
            // *target_mut_ref = data;
    
        } else {
            // 数据跨页，逐字节写入
            // 将 data 转成字节切片
            // 安全：data 是有效的 T 实例，将其视作字节序列
            // 对 POD 类型安全；对非 POD 可能有风险
            let data_bytes: &[u8] = {
                // 确保 data 生命周期足够长
                let data_ptr: *const T = &data;
                unsafe { slice::from_raw_parts(data_ptr as *const u8, data_size) }
            };
    
            let mut current_va = start_va;
            for i in 0..data_size {
                // 翻译每个字节的虚拟地址
                // 效率低但必要，因虚拟地址可能映射不连续
                let byte_pa = page_table
                    .translate_va(current_va)
                    .ok_or(PutDataError::TranslationFailed(current_va))?;
    
                // 写入字节
                // 安全：byte_pa 为已翻译的物理地址，调用者保证可写
                let dest_byte_ptr = byte_pa.get_mut::<u8>();
                unsafe { ptr::write(dest_byte_ptr, data_bytes[i]) };
    
                current_va = current_va + 1;
            }
    
            // `data` 在此作用域结束时会被丢弃
            // 因为我们使用了一个指向 data 的切片，所以 data 生命周期保持到切片结束
            // 对于非 Copy 类型，mem::forget(data) 会阻止调用其析构函数
            // 这里没有调用 mem::forget，因此 drop 会正常调用
            // 逐字节复制对于非 Copy 类型存在析构语义风险，需要注意
        }
    
        Ok(())
    }

/// 处理页错误陷阱（存储、加载、指令页错误）目前只有mmap 懒分配的逻辑
pub async fn handle_page_fault(
    &mut self,
    stval: usize,
    is_write:bool,
) -> Result<bool, PageFaultError> {

    let fault_va = VirtAddr::from(stval);
    let vpn = fault_va.floor();
    let va: VirtAddr = vpn.into();
    // 先只查找，不借用：获取起始页号
    let start = self.areatree.find_area(vpn);

    // self.areatree.debug_print();
    trace!("[mmap_page_fault] handle page fault at va:{:#x},vpn:{:#x},start_vpn:{:#x}", fault_va.0, vpn.0, start.map_or(0, |v| v.0));
    // 1. 找不到映射区 → AreaNotFound
    let start_vpn = if let Some(v) = start {
        v
    } else {
        // self.areatree.debug_print();
        return Err(PageFaultError::AreaNotFound);
    };

    
    
    let MemorySet { areatree, page_table, .. } = &mut *self;
    // areatree.debug_print();
    let area = areatree.get_mut(&start_vpn).unwrap();

    
   let area_type = &area.area_type;
  if area_type != &MapAreaType::Mmap && area_type != &MapAreaType::Stack {
        // 2. 如果不是 mmap 区域 → NotMmapType
        return Err(PageFaultError::NotMmapType);
    }

    // 3. 如果页范围为空，或该页已经被分配 → RangeEmptyOrAlreadyAllocated
    if area.vpn_range.empty()  {
        return Err(PageFaultError::RangeEmpty);
    }

    if !area.allocated(vpn) {
        // 该页已经被分配



    // 4. 如果 vpn 在范围内，则进行懒分配处理
if area.vpn_range.contains(vpn) {
    trace!("[mmap_page_fault] lazy allocate page for vpn");
        // 映射一个页（lazy allocate）
        area.map_one(page_table, vpn).expect("no memery ");

        if let Some(mmap_file) = &area.fd {
            let file = mmap_file.file.file().expect("file mmap should be normal file");
            // 保存旧的文件偏移，以便读完后恢复
            let old_offset = file.lseek(0, SEEK_CUR).unwrap();

            let start_addr: VirtAddr = start_vpn.into();
            let user_buff = UserBuffer {
                buffers: translated_byte_buffer(
                    page_table.token(),
                    va.0 as *const u8,
                    PAGE_SIZE,
                ),
            };
            // 定位到文件中对应页的偏移
            file.lseek(
                (va.0 - start_addr.0 + mmap_file.offset) as isize,
                SEEK_SET,
            )
            .expect("mmap_page_fault should not fail");

            // 实际从文件中读取到用户页
            file.read(user_buff).await.unwrap();

            // 恢复旧偏移
            file.lseek(old_offset as isize, SEEK_SET)
                .expect("mmap_page_fault should not fail");
        }

        // 最后刷新 TLB
        flush_all();
        trace!(
            "page alloc success area:{:#x}-{:#x}  addr:{:#x}",
            area.vpn_range.get_start().0,
            area.vpn_range.get_end().0,
            stval
        );
        return Ok(true)
    } else {
        // 4. 虚拟页号在映射区外 → VpnNotHandled
        unreachable!();
    }



}else {
    if let Some(pte) =  page_table.find_pte(vpn){
      match pte.is_cow()||is_write{
            true=>{
    
             let ref_count=Arc::strong_count( area.data_frames.get(&vpn).expect("cow page should have frame"));
   if ref_count > 1 {
    trace!("[mmap_page_fault] cow allocate page for vpn,pte:{:#? }",pte.flags());

    let src = &mut page_table.translate(vpn).unwrap().ppn().get_bytes_array()[..PAGE_SIZE];
    area.unmap_one(page_table, vpn);
    area.map_one(page_table, vpn).unwrap();
    let dst = &mut page_table.translate(vpn).unwrap().ppn().get_bytes_array()[..PAGE_SIZE];
    dst.copy_from_slice(src);
    {
        let mut new_pte = page_table.translate(vpn)
                             .expect("mapped just now");
        new_pte.un_cow();    
    }

        flush_all();
        return Ok(true);
        // 如果引用计数大于1，说明是共享页，需要进行COW
        // 这里需要克隆一份页帧
        // 更新页表项为新页帧
    } else {
        // 如果引用计数为1，说明是私有页，不需要COW
    trace!("[mmap_page_fault] cow not allocate page for vpn,pte:{:#? }",pte.flags());

            pte.un_cow();

        flush_all();
        return Ok(true);
    }
            }
            _=>return Ok(false),
      }

 }
 else{
    unimplemented!();
 };

}

   




}
/// Translate&Copy a ptr[u8] array end with `\0` to a `String` Vec through page table
pub  async fn safe_translated_str(&mut self, ptr: *const u8) -> String {
    let mut string = String::new();
    let mut va = ptr as usize;
    loop {
        let ch: u8 =
            *(KernelAddr::from(   
              self.safe_translate_va(VirtAddr::from(va)).await.unwrap()
        ).get_mut());
        if ch == 0 {
            break;
        }
        string.push(ch as char);
        va += 1;
    }
    string
}

pub async fn safe_translated_refmut<'a, T>(
    &'a mut self,
    ptr: *mut T,
) -> TemplateRet<&'a mut T>
where
    T: 'a,
{
    let pa = self.safe_translate_va(VirtAddr::from(ptr as usize)).await;
    match  pa {
        Some(pa)=> Ok( unsafe { &mut *pa.get_mut_ptr() }),
        None => Err(SysErrNo::EFAULT),
    }
                
}

// Translates a virtual address `ptr` from the address space identified by `token`
/// and returns a reference to `T`.
///
/// # Panics
/// - If address translation fails.
/// - If the data of type `T` at `ptr` would cross a page boundary (as this is not supported
///   for direct reference return).
///
/// # Safety
/// - Caller must ensure `token` is valid.
/// - Caller must ensure `ptr` points to a valid, readable memory location for `T`
///   within a single page in the target address space.
/// - The lifetime of the returned reference is tied to the underlying physical mapping.
///   Using `'static` here is very strong and implies the mapping is permanent.
///   Consider a shorter, more appropriate lifetime if possible.
pub async  fn safe_get_target_ref<'a, T>(&mut self, ptr: *const T) -> Result<&'a T, TranslateError> 

where
    T: 'a,
{

    let va = VirtAddr::from(ptr as usize);
    let size = core::mem::size_of::<T>();
    if size == 0 {
        // For Zero-Sized Types, a dangling pointer is fine as long as it's aligned.
        // However, we still need to ensure the concept of "location" is valid.
        // Translating the VA is a good check.
        // We can return a well-aligned dangling pointer cast to &T.
        self.safe_translate_va(va).await.ok_or(TranslateError::TranslationFailed(va))?;
        // SAFETY: For ZSTs, creating a reference from a dangling but aligned pointer is allowed.
        // return Ok(unsafe { &*(core::ptr::null::<T>() as *const T) }); // Or use ptr::NonNull::dangling()
        return Ok(unsafe { &*core::ptr::NonNull::dangling().as_ptr() });
    }

    let start_pa = self.safe_translate_va(va).await.ok_or(TranslateError::TranslationFailed(va))?;

    // Check for cross-page boundary for the physical address
    // (va.as_usize() / PAGE_SIZE) != ((va.as_usize() + size - 1) / PAGE_SIZE)
    // Better: start_pa.floor() != (start_pa + size - 1).floor()
    if size > 0 && start_pa.floor() != (start_pa + (size - 1)).floor() {
        return Err(TranslateError::DataCrossesPageBoundary);
    }

    // TODO: Add permission checks (e.g., readability) from page table entry if possible @Heliosly.

    // SAFETY: Caller ensures validity. We've checked translation and single-page constraint.
    // The lifetime 'a should be tied to the validity of the mapping.
    // Using 'static is dangerous unless the mapping is truly static.
    Ok(unsafe { &*start_pa.get_ptr::<T>() }) // Assuming PhysAddr::as_ptr() returns *const T
}



pub async fn safe_translated_byte_buffer(&mut self, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();

        let ppn = self.safe_translate(vpn).await.unwrap().ppn();

        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}

pub async fn mprotect(&mut self, start: VirtAddr, size: usize, flags: MapPermission) {
    
     // `drain_filter()` out the overlapped areas first.

     let end = start + size;
     let end_vpn = end.ceil();
     let start_vpn = start.floor();

     let mut overlapped_area: Vec<(usize, MapArea)> = Vec::new();
     let mut prev_area: BTreeMap<VirtPageNum, MapArea> = BTreeMap::new();
     for _ in 0..self.areatree.len() {
        let (idx, area) = self.areatree.pop_first().unwrap();
        if area.overlap_with(start_vpn, end_vpn) {
            overlapped_area.push((idx.0, area));
        } else {
            prev_area.insert(idx, area);
        }
    }
    self.areatree.areas = prev_area;
    for (_, mut area) in overlapped_area {
        if area.contained_in(start_vpn, end_vpn) {
            // update whole area
            area.update_flags(flags, &mut self.page_table);
        } else if area.strict_contain(start_vpn, end_vpn) {
            // split into 3 areas, update the middle one
            let (mut mid, right) = area.split3(start_vpn, end_vpn).await;
            mid.update_flags(flags, &mut self.page_table);

            assert!(self.areatree.insert(mid.start_vpn(), mid).is_none());
            assert!(self.areatree.insert(right.start_vpn(), right).is_none());
        } else if start_vpn <= area.start_vpn() && area.start_vpn() < end_vpn {
            // split into 2 areas, update the left one
            let right = area.split(end_vpn).await;
            area.update_flags(flags, &mut self.page_table);

            assert!(self.areatree.insert(right.start_vpn(), right).is_none());
        } else {
            // split into 2 areas, update the right one
            let mut right = area.split(start_vpn).await;
            right.update_flags(flags, &mut self.page_table);

            assert!(self.areatree.insert(right.start_vpn(), right).is_none());
        }

        assert!(self.areatree.insert(area.start_vpn(), area).is_none());
    }




}
 /// 手动分配 
 pub async fn manual_alloc_type_for_lazy<T: Sized>(&mut self, obj: *const T) -> GeneralRet {
    let start = obj as usize;
    let end =match start.checked_add(core::mem::size_of::<T>() - 1)  {
        Some(s) => s,
        None => return Err(SysErrNo::EINVAL),
    };
    self.manual_alloc_range_for_lazy(start.into(), end.into()) .await?;
    Ok( ())
}
/// mremap: change the size of a mapping, potentially moving it at the same time.
pub async fn mremap(&mut self, old_start: VirtAddr, old_size: usize, new_size: usize,flags: MremapFlags) -> SyscallRet{
   

    
    let old_start_vpn= old_start.floor();
    let start = self.areatree.find_area(old_start_vpn);
    


    let addr: usize = match start {
        Some(start) => {
             let area_type;
            let old_area_vpn_range;
          
            let lacklen= (new_size as isize)-(old_size as isize);  
            let new_end= old_start.0+new_size;
            {
            
            // self.areatree.debug_print();
            let old_area=self.areatree.get(&start).unwrap();
             area_type = old_area.area_type;
             old_area_vpn_range=old_area.vpn_range;
            //  old_area.debug_print();
           }

             info!(
        "[mremap] old_start: {:?},  end: {:x},this area start:{:#?},lacklen:{:#x}",
        old_start, new_end as usize,VirtAddr::from(start),lacklen,
    );
            if lacklen ==0 {
                return Ok(old_start.0);
            }    
            if lacklen<0{
               self.shrink_to(start.into(), VirtAddr::from(new_end as usize) );
                
               old_start.0

            }
            else{
               let vpn= self.areatree.find_gap_from( VirtPageNum::from(new_end>>PAGE_SIZE_BITS), new_size>>PAGE_SIZE_BITS);
               let allocated=if let Some(vpn)=vpn{
               if vpn == old_start.floor(){
                  self.append_to(start.into(), VirtAddr::from(new_end as usize) );
                  true
               }else{
                 false
               }
               }
               else{
                false
               };
               if !allocated&&flags.contains(MremapFlags::MAYMOVE){
                   if let Some(vpn)= self.areatree.find_gap_from( VirtPageNum(MMAP_PGNUM_TOP), new_size>>PAGE_SIZE_BITS){
                    let end_vpn =VirtPageNum::from( vpn.0+(new_size>>PAGE_SIZE_BITS));
                    // info!("statt:{:?},end:{:?}",vpn,end_vpn );
                    let new_area =MapArea::new_by_vpn(
                        vpn,
                        end_vpn,
                        MapType::Framed,
                           MapPermission::U|MapPermission::W|MapPermission::R,
                        area_type
                    );
                    let new_area_vpn_range=new_area.vpn_range;
                    self.push(new_area, None)?;
                    flush_all();

                    let pagetable = &self.page_table;
                    for (old_vpn, new_vpn) in
                    old_area_vpn_range.iter().zip(new_area_vpn_range.iter())
                {
                    // 从旧 vpn 读页号，再从新 vpn 写页号
                    let src_ppn = pagetable.translate(old_vpn).unwrap().ppn();
                    let dst_ppn = pagetable.translate(new_vpn).unwrap().ppn();
                
                    // 拷贝一页数据
                    dst_ppn
                        .get_bytes_array()
                        .copy_from_slice(src_ppn.get_bytes_array());
                }

                    self.munmap(old_area_vpn_range.get_start(),old_area_vpn_range.get_end());
                    vpn.0<<PAGE_SIZE_BITS
                   }                    
                   else{
                    
                      return Err(SysErrNo::ENOMEM);
                   }

                  
               }

               else{

                
                old_start.0

               }


                
            }

        }
        None => {
            
    debug!("[mremap] Can't find area ");
           return  Err(SysErrNo::EFAULT);
        },
    };

    debug!("[mremap] return addr: 0x{:x}", addr);
    Ok(addr)
}
/// Handles the MADV_DONTNEED advice.
///
/// This function iterates through the given virtual page range and decommits
/// the pages. Decommitting means unmapping the page from the page table and
/// deallocating the physical frame it was pointing to.
///
/// The associated Virtual Memory Area (VMA) is NOT removed, so a subsequent
/// access to this memory region will trigger a page fault, and the kernel
/// can re-allocate a new, zeroed page on demand.
pub fn madvise_dontneed(&mut self, start_vpn: VirtPageNum, end_vpn: VirtPageNum) {
    // 遍历指定范围内的每一个虚拟页
    for vpn in VPNRange::new(start_vpn, (end_vpn.0+ 1).into()) {
        // 查找该虚拟页对应的页表项 (PTE)
        if let Some(pte) = self.page_table.find_pte(vpn) {
            // 检查页表项是否有效（即是否映射到了一个物理页）
            if pte.is_valid() {
                // 如果有效，则：
                // 1. 获取其指向的物理页号
                let ppn = pte.ppn();

                // 2. 将页表项清空，解除映射关系
                // Setting the entry to 0 clears all flags, including the Valid bit.
                *pte = PageTableEntry::new(0.into(), super::PTEFlags::empty());

                // 3. 释放对应的物理页帧
                // The frame allocator will add this physical page back to the free list.
                match self.areatree.find_area(vpn){
                    Some(area) => {
                        if let Some(area_mut) = self.areatree.get_mut(&area) {
                            if let Some(_frame_tracker) = area_mut.data_frames.remove(&vpn) {
                                
                            } else {
                                warn!("Frame tracker not found for vpn: {:?}", vpn);
                            }
                        }
                    }
                    None => {
                        unreachable!();
                    }
                }
                
                // 4. 刷新 TLB
                // This is crucial! It ensures that the CPU's cached translation
                // for this virtual page is invalidated. Otherwise, the CPU might
                // still be able to access the old physical memory.
            }
            // 如果 pte 无效 (pte.is_valid() is false)，说明该页尚未分配物理内存，
            // 我们什么也不用做。
        }
        // 如果 find_pte 返回 None，说明连中间的页表都还没创建，
        // 同样也什么都不用做。
    }
 flush_all();
}
}

#[allow(unused)]
pub fn remap_test() {
    use crate::config::KERNEL_DIRECT_OFFSET; // 引入内核直接映射偏移量

    info!("[kernel]:remap testing");
    let page_table = PageTable::from_token(*KERNEL_PAGE_TABLE_TOKEN);
    let mid_text: VirtAddr = (stext as usize + (etext as usize - stext as usize) / 2).into();
    let mid_rodata: VirtAddr =
        (srodata as usize + (erodata as usize - srodata as usize) / 2).into();
    let mid_data: VirtAddr = (sdata as usize + (edata as usize - sdata as usize) / 2).into();

    // --- 原有测试保持不变 ---
    assert!(!page_table
        .translate(mid_text.floor())
        .unwrap()
        .writable());
    debug!("text pass");
    assert!(!page_table
        .translate(mid_rodata.floor())
        .unwrap()
        .writable());
    debug!("rodata pass");

    #[cfg(target_arch = "riscv64")]
    assert!(!page_table
        .translate(mid_data.floor())
        .unwrap()
        .executable());

    // --- 新增：验证直接映射区域 ---
    debug!("Verifying direct-mapped physical memory area...");

    // 1. 定义测试范围
    //    我们从 _ekernel (内核静态数据末尾的虚拟地址) 开始
    //    选取一个在该区域内的随机点进行测试，避免边界问题。
    //    这里的 ekernel 和 MEMORY_END 都是虚拟地址，正如你之前确认的。
    let direct_map_start_va: VirtAddr = (_ekernel as usize).into();
    let direct_map_end_va: VirtAddr = (MEMORY_END as usize).into();

    // 2. 选取一个测试点 (例如，区域的中间点)
    let test_va: VirtAddr =
        (direct_map_start_va.0 + (direct_map_end_va.0 - direct_map_start_va.0) / 2).into();
    // 3. 预期物理地址
    //    根据直接映射的定义，物理地址应该是虚拟地址减去偏移。
    let expected_pa: usize = test_va.0 - KERNEL_DIRECT_OFFSET;

    info!("  Testing direct map VA: {:#x}", test_va.0);
    info!("  Expected physical address: {:#x}", expected_pa);

    // 4. 使用页表进行翻译
    match page_table.translate_va(test_va) {
        Some(translated_pa) => {
            info!("  Translated physical address: {:#x}", translated_pa.0);
            
            // 5. 验证翻译结果
            assert_eq!(
                translated_pa.0,
                expected_pa,
                "Direct map translation FAILED! VA {:#x} translated to {:#x}, but expected {:#x}",
                test_va.0,
                translated_pa.0,
                expected_pa
            );
            
            debug!("Direct map translation verification PASSED.");
        }
        None => {
            // 如果翻译失败，直接 panic，因为这块区域必须被映射。
            panic!(
                "Direct map translation FAILED! VA {:#x} is not mapped in the page table.",
                test_va.0
            );
        }
    }

    println!("remap_test passed!");
}
