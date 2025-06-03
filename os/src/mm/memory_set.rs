//! Implementation of [`MapArea`] and [`MemorySet`].
use crate::config::{ DL_INTERP_OFFSET, KERNEL_DIRECT_OFFSET, PAGE_SIZE_BITS};
use crate::fs::{map_dynamic_link_file, open_file, File, OpenFlags, NONE_MODE};
use crate::mm::{ translated_byte_buffer, FrameTracker, UserBuffer, KERNEL_PAGE_TABLE_TOKEN};
use crate::task::aux::{Aux, AuxType};
use crate::utils::error::{GeneralRet, SysErrNo, TemplateRet};
use super::area::{MapArea, MapAreaType, MapPermission, MapType, VmAreaTree};
use super::page_table::{self, PutDataError, PutDataRet};
use super::{flush_tlb, KernelAddr, MmapFlags, PhysAddr, StepByOne, TranslateError, VirtAddr, VirtPageNum};
use super::{PageTable, PageTableEntry};
use crate::config::{MEMORY_END, MMIO, PAGE_SIZE,/*  TRAMPOLINE, TRAP_CONTEXT_BASE,*/};
use alloc::collections::btree_map::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use lwext4_rust::bindings::{SEEK_CUR, SEEK_SET};
use riscv::register::scause::{Exception,  Trap};
use xmas_elf::ElfFile;
use core::arch::asm;
use core::{ptr, slice};
use core::sync::atomic::AtomicBool;
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
    // fn spercpu();
    // fn epercpu();
    // fn strampoline();
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
    RangeEmptyOrAlreadyAllocated,
    /// 虚拟页号虽在 area.vpn_range 内，但不满足懒分配条件
    VpnNotHandled,
}
impl From<PageFaultError> for SysErrNo {
    fn from(err: PageFaultError) -> SysErrNo {
        match err {
            PageFaultError::AreaNotFound => SysErrNo::EFAULT,
            PageFaultError::NotMmapType => SysErrNo::EINVAL,
            PageFaultError::RangeEmptyOrAlreadyAllocated => SysErrNo::EBUSY,
            PageFaultError::VpnNotHandled => SysErrNo::EINVAL,
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
        if area.allocated(r0)
    {
        for vpn in r0.0.. r1.0 {
            self.page_table.unmap(VirtPageNum(vpn));
        }
    }
        flush_tlb();
    }

   
 }
    
    /// 复制逻辑段内容
    pub fn clone_area(&mut self, start_vpn: VirtPageNum, another:&MemorySet) {
        if let Some(area) = another
            .areatree.get(&start_vpn)
            
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
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission, area_type),
            None,

        );
    }
    /// remove a area
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
       
        if let Some((_, mut area)) = self.areatree.remove_entry(&start_vpn) {
            area.unmap(&mut self.page_table);
        }
    }

    ///有页内偏移的push
    fn push_with_offset(&mut self, mut map_area: MapArea, offset: usize, data: Option<&[u8]>)  {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data, offset);
        }
        self.areatree.push(map_area);
    }
    /// Add a new MapArea into this MemorySet.
    /// Assuming that there are no conflicts in the virtual address
    /// space.
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data,0);
        }
        self.areatree.push(map_area);
    }

   
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
    fn map_elf(&mut self, elf: &ElfFile, offset: VirtAddr) -> (VirtPageNum, VirtAddr,usize) {
        let mut tls=0;
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
                );
            } 
            else  if ph.get_type().unwrap()== xmas_elf::program::Type::Tls  {

                
                // let mem_size = ph.mem_size() as usize;
                // let file_size = ph.file_size() as usize;
                // let file_offset = ph.offset() as usize;
                // let tls_align = ph.align() as usize;
                // if tls_align > PAGE_SIZE {
                //     // 这需要更复杂的对齐分配逻辑，这里我们先 panic 或报错
                //     // 实际上，大多数情况下 ph.align() 不会大于 PAGE_SIZE
                //     log::error!("[map_elf_tls] PT_TLS p_align ({}) > PAGE_SIZE ({}) - UNSUPPORTED ALIGNMENT", tls_align, PAGE_SIZE);
                //     // return Err("Unsupported TLS alignment"); // 或者 panic
                //     tls = 0; // 表示失败或不支持
                //     continue;
                // }
                // // TLS 段权限一般是可读写，且不执行
                // let map_perm = MapPermission::U | MapPermission::R | MapPermission::W;
                // let npages=(mem_size + PAGE_SIZE - 1) / PAGE_SIZE;
                // let start_vpn=self.areatree.alloc_pages(npages).unwrap();
                // let end_vpn =VirtPageNum::from(start_vpn.0+npages);
                // // 创建映射区域
                // let map_area = MapArea::new(
                //     start_vpn.into(),
                //     end_vpn.into(),
                //     MapType::Framed,
                //     map_perm,
                //     MapAreaType::Tls,
                // );
                // // 从文件中复制初始化数据（tdata）
                // let data = &elf.input[file_offset..file_offset + file_size];
            
                // // 把映射区域加入内存管理，同时初始化内存内容
                // self.push_with_offset(map_area, 0, Some(data));
            
                // tls=VirtAddr::from(start_vpn).0+mem_size;

                // warn!("[map_elf]tls:{:#x}",tls);
                // if tls % tls_align != 0 {
                //     // 这种情况理论上不应该发生，如果 tls_align <= PAGE_SIZE 且是2的幂
                //     log::warn!("[map_elf_tls] Page-aligned TLS base 0x{:x} does not meet align 0x{:x}", tls, tls_align);
                //     // 这里可能需要失败，或者尝试在分配的页面内向上调整基址（如果空间允许）
                //     // 但这会使 tp 的计算复杂化，因为你可能不再使用整个分配的页面作为 TLS 块。
                //     // 最好的办法是确保分配器返回的地址本身就满足对齐。
                //     // 为了演示，我们先假设它满足，或者这个问题需要单独解决。
                // } 
                // // 记录 TLS 基址，方便 TLS 访问
                       }
        }
        (max_end_vpn, header_va.into(),tls)
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
            trace!("no dl");
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
        let (max_end_vpn, head_va,_tls) = memory_set.map_elf(&elf, VirtAddr(0));
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
   
    fn push_with_given_frames(&mut self, mut map_area: MapArea, frames: Vec<Arc<FrameTracker>>) {
        map_area.map_given_frames(&mut self.page_table, frames);
        self.areatree.push(map_area);
    }
      /// Create a new address space by copy code&data from a exited process's address space.
      pub fn from_existed_user(user_space: &mut Self) -> Self {
        let mut memory_set = Self::new_from_kernel();

        let mut old_page_table_ref = &mut user_space.page_table; 
        let old_areatree_mut_ref = &mut user_space.areatree; 
        // memory_set.map_trampoline();

        for (_vpn_key, area) in old_areatree_mut_ref.iter_mut().filter(|(_, a)| a.area_type != MapAreaType::Stack) {
            let  new_area = MapArea::from_another(area);

            if area.area_type == MapAreaType::Mmap  {
                if area.mmap_flags.contains(MmapFlags::MAP_SHARED) {
                    // 对于共享映射:
                    let frames:  Vec<Arc<FrameTracker>> = area.data_frames.values().cloned().collect();
                    memory_set.push_with_given_frames(new_area, frames);

                    continue; 

                    }
                   

                   
                

                // 为了解决第一个所有权冲突：先收集需要操作的VPN
                let mut vpns_to_map_one = Vec::new();
                for vpn_key in area.vpn_range { // 不可变借用 area.data_frames
                   
                    if !area.allocated(vpn_key){ // 不可变借用 area
                        
                        // println!("vpn:{:#x} not allocated",vpn_key.0);
                        vpns_to_map_one.push(vpn_key);
                    }
                }
                // area.data_frames.iter() 的借用在这里结束
                for vpn_to_map in vpns_to_map_one {
                    area.map_one(&mut old_page_table_ref, vpn_to_map);
                }
            }

            let area_to_push = new_area; 
            memory_set.push(area_to_push, None);
            // area.debug_print();

            // copy data from another space
            for vpn in area.vpn_range {
                let src_ppn = old_page_table_ref.translate(vpn).unwrap().ppn();
                let dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                // println!("copying vpn:{:#x} src_ppn:{:#x} dst_ppn:{:#x}",vpn.0,src_ppn.0,dst_ppn.0);
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
   
   pub async  fn safe_translate(&mut self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        // Check if the page table is valid
        
        let va = VirtAddr::from(vpn).0;
        loop{
        match self.page_table.translate(vpn) {
            None => {
               if self.handle_page_fault(va).await.is_ok(){
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
                   if self.handle_page_fault(va).await.is_ok(){
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

    ///Remove all `MapArea`
    pub fn recycle_data_pages(&mut self) {
        self.page_table.clear();
        self.areatree.clear();
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
        assert!(start_va < end_va);
        
        let mut vpn = start_va.floor();
        let end=end_va.ceil();
     
        while vpn < end {
           match self.handle_page_fault(VirtAddr::from(vpn).0 ).await{
            
            Err(e) => 
{
    // println!("[ manual_alloc_range_for_lazy] err:{:#?},vpn:{:#x}",e,vpn.0);
                if matches!(e, PageFaultError::NotMmapType | PageFaultError::RangeEmptyOrAlreadyAllocated) {
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
) -> Result<bool, PageFaultError> {
    let fault_va = VirtAddr::from(stval);
    let vpn = fault_va.floor();
    let va: VirtAddr = vpn.into();
    // 先只查找，不借用：获取起始页号
    let start = self.areatree.find_area(vpn);
    // 1. 找不到映射区 → AreaNotFound
    let start_vpn = if let Some(v) = start {
        v
    } else {
        // println!("vpn:{:#x}",vpn.0);
        // self.areatree.debug_print();
        return Err(PageFaultError::AreaNotFound);
    };

    let MemorySet { areatree, page_table, .. } = &mut *self;
    let area = areatree.get_mut(&start_vpn).unwrap();

    // 2. 不是 mmap 类型 → NotMmapType
    if area.area_type != MapAreaType::Mmap {
        return Err(PageFaultError::NotMmapType);
    }

    // 3. 如果页范围为空，或该页已经被分配 → RangeEmptyOrAlreadyAllocated
    if area.vpn_range.empty() || area.allocated(vpn) {
        return Err(PageFaultError::RangeEmptyOrAlreadyAllocated);
    }

    // 4. 如果 vpn 在范围内，则进行懒分配处理
    if area.vpn_range.contains(vpn) {
        // 映射一个页（lazy allocate）
        area.map_one(page_table, vpn);

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
        flush_tlb();

        trace!(
            "page alloc success area:{:#x}-{:#x}  addr:{:#x}",
            area.vpn_range.get_start().0,
            area.vpn_range.get_end().0,
            stval
        );
        Ok(true)
    } else {
        // 4. 虚拟页号在映射区外 → VpnNotHandled
        unreachable!();
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
    let end = start + core::mem::size_of::<T>() - 1;
    self.manual_alloc_range_for_lazy(start.into(), end.into()) .await?;
    Ok( ())
}


}

/// remap test in kernel space
#[allow(unused)]
pub fn remap_test() {
    
    info!("[kernel]:remap testing");
    let page_table=PageTable::from_token(*KERNEL_PAGE_TABLE_TOKEN);
    let mid_text: VirtAddr = (stext as usize + (etext as usize - stext as usize) / 2).into();
    let mid_rodata: VirtAddr =
        (srodata as usize + (erodata as usize - srodata as usize) / 2).into();
    let mid_data: VirtAddr = (sdata as usize + (edata as usize - sdata as usize) / 2).into();
    assert!(!
        page_table
        .translate(mid_text.floor())
        .unwrap()
        .writable(),);
    debug!("text pass");
    assert!(!
        page_table
        .translate(mid_rodata.floor())
        .unwrap()
        .writable(),);
    debug!("rodata pass");
    assert!(!page_table
        .translate(mid_data.floor())
        .unwrap()
        .executable(),);
    println!("remap_test passed!");
}

