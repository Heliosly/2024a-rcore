//! Implementation of [`PageTableEntry`] and [`PageTable`].



use super::{KernelAddr, MapPermission, KERNEL_PAGE_TABLE_PPN};
use super::{frame_alloc, FrameTracker, PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use crate::config::{self, PAGE_SIZE};
use crate::timer::get_time_ticks;
use crate::utils::error::{ SysErrNo, TemplateRet};
bitflags::bitflags! {
    /// Page-table entry flags.
    pub struct PTEFlags: usize {
        /// Whether the PTE is valid.
        const V =   1 << 0;
        /// Whether the page is readable.
        const R =   1 << 1;
        /// Whether the page is writable.
        const W =   1 << 2;
        /// Whether the page is executable.
        const X =   1 << 3;
        /// Whether the page is accessible to user mode.
        const U =   1 << 4;
        /// Designates a global mapping.
        const G =   1 << 5;
        /// Indicates the virtual page has been read, written, or fetched from
        /// since the last time the A bit was cleared.
        const A =   1 << 6;
        /// Indicates the virtual page has been written since the last time the
        /// D bit was cleared.
        const D =   1 << 7;
    }
}
#[derive(Copy, Clone)]
#[repr(C)]
/// page table entry structure
pub struct PageTableEntry {
    /// bits of page table entry
    pub bits: usize,
}

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = (1 << 54) - (1 << 10); // bits 10..54
    /// Create a new page table entry
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    /// Create an empty page table entry
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    /// Get the physical page number from the page table entry
    pub fn ppn(&self) -> PhysPageNum {
        (self.bits >> 10 & ((1usize << 44) - 1)).into()
    }
    pub fn set_ppn(&mut self,paddr : PhysAddr){
        self.bits = (self.bits & !Self::PHYS_ADDR_MASK)
        | ((paddr.0>> 2) & Self::PHYS_ADDR_MASK);
    }
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits_truncate(self.bits )
    }
    /// The page pointered by page table entry is valid?
    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is readable?
    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is writable?
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is executable?
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
  pub  fn set_flags(&mut self, flags: MapPermission) {
        let flags = PTEFlags::from(flags) | PTEFlags::A | PTEFlags::D;
        debug_assert!(flags.intersects(PTEFlags::R | PTEFlags::X));
        self.bits = (self.bits & Self::PHYS_ADDR_MASK) | flags.bits() as usize;
    }
   pub fn is_huge(&self) -> bool {
        PTEFlags::from_bits_truncate(self.bits).intersects(PTEFlags::R | PTEFlags::X)
    }
    
   
}


impl From<MapPermission> for PTEFlags {
    fn from(f: MapPermission) -> Self {
        if f.is_empty() {
            return Self::empty();
        }
        let mut ret = Self::V;
        if f.contains(MapPermission::R) {
            ret |= Self::R;
        }
        if f.contains(MapPermission::W) {
            ret |= Self::W;
        }
        if f.contains(MapPermission::X) {
            ret |= Self::X;
        }
        if f.contains(MapPermission::U) {
            ret |= Self::U;
        }
        ret
    }
}

/// 页表操作失败的错误类型。
#[derive(Debug)]
pub enum PagingError {
    /// 无法分配内存。
    NoMemory,
    /// 地址未与页面大小对齐。
    NotAligned,
    /// 映射不存在。
    NotMapped,
    /// 映射已存在。
    AlreadyMapped,
    /// 页表条目表示一个大页面，但目标物理帧的大小为 4K。
    MappedToHugePage,
    
  
}
/// page table structure
pub struct PageTable {

    root_ppn: PhysPageNum,
    frames: Vec<Arc<FrameTracker>>,
}

#[repr(usize)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
/// 页面的大小。
pub enum PageSize {
    /// 4 千字节的大小 (2<sup>12</sup> 字节)。
    Size4K = 0x1000,
    /// 2 兆字节的大小 (2<sup>21</sup> 字节)。
    Size2M = 0x20_0000,
    /// 1 吉字节的大小 (2<sup>30</sup> 字节)。
    Size1G = 0x4000_0000,
}


/// 用于页表操作的特殊 Result 类型。
pub type PagingResult<T = ()> = Result<T, PagingError>;



/// Assume that it won't oom when creating/mapping.
impl PageTable {
    /// 更新一个连续虚拟内存区域的映射标志。
    /// 在使用 [`PageTable64::map_region`] 之前，该区域必须已经被映射，否则会返回一个错误。
    pub fn update_region(
        &mut self,
        mut vaddr: VirtAddr,
        size: usize,
        flags: MapPermission,
    ) -> PagingResult {
        let end = vaddr + size;
        while vaddr < end {
            let page_size = self.update(vaddr, None, Some(flags))?;
            vaddr.0 += page_size as usize;
        }
        Ok(())
    }

    /// 更新从 `vaddr` 开始的映射的目标或标志。如果相应的参数是 `None`，则不会更新。
    ///
    /// 返回映射的页面大小。
    ///
    /// 如果映射不存在，则返回 [`Err(PagingError::NotMapped)`](PagingError::NotMapped)。
    pub fn update(
        &mut self,
        vaddr: VirtAddr,
        paddr: Option<PhysAddr>,
        flags: Option<MapPermission>,
    ) -> PagingResult<PageSize> {
        let vpn= vaddr.floor();
       let pte= match  self.find_pte(vpn){
            Some(f) => f,
            None => return Err(PagingError::NotMapped),
        };
        // if pte.ppn() == 0.into() {
        //     return Ok();
        // }
        if let Some(paddr) = paddr {
            pte.set_ppn(paddr);
        }
        if let Some(flags) = flags {
            pte.set_flags(flags);
        }
        //现在只能映射4k页面
        Ok(PageSize::Size4K)
    }
    ///clear frame 
    pub fn clear(&mut self) {
        self.frames.clear();
    }
    ///Create new PageTable from global kernel space
    pub fn new_from_kernel() -> Self {
        let frame = frame_alloc().unwrap();
        let global_root_ppn = *KERNEL_PAGE_TABLE_PPN ;

        // Map kernel space
        // Note that we just need shallow copy here
        let kernel_start_vpn = VirtPageNum::from(config::KERNEL_PGNUM_OFFSET);
        let level_1_index = kernel_start_vpn.indexes()[0];
        
        //截断高地址页表以访问
        frame.ppn().get_pte_array()[level_1_index..]
            .copy_from_slice(&global_root_ppn.get_pte_array()[level_1_index..]);

        // the new pagetable only owns the ownership of its own root ppn
        PageTable {
            root_ppn: frame.ppn(),
            frames: vec![frame],
        }
    }
    pub fn root_ppn(&self)->PhysPageNum{
        self.root_ppn
    }
    /// Create a new page table
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        debug!("frame: {:#x}",frame.ppn().0);
        PageTable {
            root_ppn: frame.ppn(),
            frames: vec![frame],
        }
    }
    /// Temporarily used to get arguments from user space.
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
    /// Find PageTableEntry by VirtPageNum, create a frame for a 4KB page table if not exist
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn(), PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }
    /// Find PageTableEntry by VirtPageNum
    pub fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                info!("pte is invalid:vpn:{:x}",vpn.0);
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }
    
    /// set the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        // if vpn.0==0x2fe7d46{
        //     println!("map vpn:{:x},ppn:{:x},flags:{:?}",vpn.0,ppn.0,flags);
        // }
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    /// remove the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }

    
    ///并不安全也许有部分页面是懒分配的页面，这样会触发内核错误todo(Heliosly)
    /// get the page table entry from the virtual page number
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }
    /// get the physical address from the virtual address
    /// va to pa
    pub fn translate_va(&self, va: VirtAddr) -> Option<PhysAddr> {
        self.find_pte(va.clone().floor()).map(|pte| {
            let aligned_pa: PhysAddr = pte.ppn().into();
            let offset = va.page_offset();
            let aligned_pa_usize: usize = aligned_pa.into();
            (aligned_pa_usize + offset).into()
        })
    }

    pub fn translate_va_with_perm(&self, va: VirtAddr, require_writable: bool) -> Result<PhysAddr, TranslateError> {
        match self.find_pte(va.clone().floor())
        {
            Some(pte) => {
              
            if require_writable && !pte.writable() {
                    return Err(TranslateError::PermissionDenied(va));
            }
            let aligned_pa: PhysAddr = pte.ppn().into();
            let offset = va.page_offset();
            let aligned_pa_usize: usize = aligned_pa.into();
       
             return  Ok((aligned_pa_usize + offset).into())
                
            },
            None => {
                return Err(TranslateError::PermissionDenied(va));
            }
        }
          


            }

        
        
        
        
            

        
    
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

/// An abstraction over a buffer passed from user space to kernel space

pub struct UserBuffer<'b> {
    /// A list of buffers
    pub buffers: Vec<&'b mut [u8]>,
}

impl<'b> UserBuffer<'b> {
    /// Constuct UserBuffer
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }
    pub fn is_empty (&self) ->bool{
          self.len()==0
    }
    /// Get the length of the buffer
    pub fn len(&self) -> usize {
        let mut total: usize = 0;
        for b in self.buffers.iter() {
            total += b.len();
        }
        total
    }
    /// 将内容数组返回
    pub fn read(&mut self, len: usize) -> Vec<u8> {
        let mut bytes = vec![0; len];
        let mut current = 0;
        for sub_buff in self.buffers.iter_mut() {
            let mut sblen = (*sub_buff).len();
            if current + sblen > len {
                sblen = len - current;
            }
            bytes[current..current + sblen].copy_from_slice(&(*sub_buff)[..sblen]);
            current += sblen;
            if current == len {
                return bytes;
            }
        }
        bytes
    }
    pub fn fill0(&mut self) -> usize {
        for sub_buff in self.buffers.iter_mut() {
            let sblen = (*sub_buff).len();
            for j in 0..sblen {
                (*sub_buff)[j] = 0;
            }
        }
        self.len()
    }

    pub fn fillrandom(&mut self) -> usize {
        //随机数生成方法： 线性计算+噪声+零特殊处理
        let mut random: u8 = (get_time_ticks() % 256) as u8;
        for sub_buff in self.buffers.iter_mut() {
            let sblen = (*sub_buff).len();
            for j in 0..sblen {
                if random == 0 {
                    random = (get_time_ticks() % 256) as u8;
                }
                random = (((random as usize) * (get_time_ticks() / 3 % 256) + 37) % 256) as u8; //生成一个字节大小的随机数
                (*sub_buff)[j] = random;
            }
        }
        self.len()
    }

    pub fn printbuf(&mut self, size: usize) {
        if size == 0 {
            return;
        }
        let mut count: usize = 0;
        for sub_buff in self.buffers.iter_mut() {
            let sblen = (*sub_buff).len();
            for j in 0..sblen {
                print!("{} ", (*sub_buff)[j]);
                count += 1;
                if count == size {
                    println!("");
                    return;
                }
            }
        }
    }

    pub fn clear(&mut self) -> usize {
        self.buffers.clear();
        self.len()
    }
    /// 将一个Buffer的数据写入UserBuffer，返回写入长度
    pub fn write(&mut self, buff: &[u8]) -> usize {
        let len = self.len().min(buff.len());
        if len == 0 {
            return len;
        }
        let mut current = 0;
        for sub_buff in self.buffers.iter_mut() {
            let mut sblen = (*sub_buff).len();
            if buff.len() > 10 {
                if current + sblen > len {
                    sblen = len - current;
                }
                (*sub_buff)[..sblen].copy_from_slice(&buff[current..current + sblen]);
                current += sblen;
                if current == len {
                    return len;
                }
            } else {
                for j in 0..sblen {
                    (*sub_buff)[j] = buff[current];
                    current += 1;
                    if current == len {
                        return len;
                    }
                }
            }
        }
        return len;
    }
    //在指定位置写入数据
    pub fn write_at(&mut self, offset: usize, buff: &[u8]) -> isize {
        //未被使用，暂不做优化
        let len = buff.len();
        if offset + len > self.len() {
            return -1;
        }
        let mut head = 0; // offset of slice in UBuffer
        let mut current = 0; // current offset of buff

        for sub_buff in self.buffers.iter_mut() {
            let sblen = (*sub_buff).len();
            if head + sblen < offset {
                continue;
            } else if head < offset {
                for j in (offset - head)..sblen {
                    (*sub_buff)[j] = buff[current];
                    current += 1;
                    if current == len {
                        return len as isize;
                    }
                }
            } else {
                //head + sblen > offset and head > offset
                for j in 0..sblen {
                    (*sub_buff)[j] = buff[current];
                    current += 1;
                    if current == len {
                        return len as isize;
                    }
                }
            }
            head += sblen;
        }
        0
    }
}

impl<'b> IntoIterator for UserBuffer<'b> {
    type Item = *mut u8;
    type IntoIter = UserBufferIterator<'b>;
    fn into_iter(self) -> Self::IntoIter {
        UserBufferIterator {
            buffers: self.buffers,
            current_buffer: 0,
            current_idx: 0,
        }
    }
}

/// An iterator over a UserBuffer
pub struct UserBufferIterator<'b> {
    buffers: Vec<&'b mut [u8]>,
    current_buffer: usize,
    current_idx: usize,
}

impl<'b> Iterator for UserBufferIterator<'b> {
    type Item = *mut u8;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_buffer >= self.buffers.len() {
            None
        } else {
            let r = &mut self.buffers[self.current_buffer][self.current_idx] as *mut _;
            if self.current_idx + 1 == self.buffers[self.current_buffer].len() {
                self.current_idx = 0;
                self.current_buffer += 1;
            } else {
                self.current_idx += 1;
            }
            Some(r)
        }
    }
}





use core::mem::MaybeUninit;
use core::{ptr,slice};
#[derive(Debug)]
pub enum PutDataError {
    /// 地址翻译失败，包含失败的虚拟地址
    TranslationFailed(VirtAddr),

    /// 访问未对齐，比如指针不满足类型的对齐要求
    UnalignedAccess,

    /// 物理内存不连续，写入要求连续内存但实际不连续
    NonContiguousPhysicalMemory,

    /// 权限错误，比如目标页不可写
    PermissionDenied,

    /// 其他通用错误
    Other(&'static str),
}

impl From<PutDataError> for SysErrNo {
    fn from(err: PutDataError) -> Self {

        warn!("[PutDataError]err:{:#?}",err);
        match err {
            PutDataError::TranslationFailed(_) => SysErrNo::ENOMEM,
            PutDataError::UnalignedAccess => SysErrNo::EFAULT,
            PutDataError::NonContiguousPhysicalMemory => SysErrNo::EFAULT,
            PutDataError::PermissionDenied => SysErrNo::EACCES,
            PutDataError::Other(_) => SysErrNo::EIO,
        }
    }
}

pub type PutDataRet= Result<(),PutDataError>;

/// 将类型为 `T` 的数据写入由 `token` 标识的目标地址空间中的虚拟地址 `ptr`。
/// 支持跨页边界的数据写入。
///
/// # 参数
/// * `token`: 目标地址空间的标识符（例如页表标识符）。
/// * `ptr`: 目标地址空间中的虚拟地址。
/// * `data`: 要写入的数据，`T` 按值传递。
///
/// # 安全性
///
/// 该函数为不安全函数，原因包括：
/// 1. 它会解引用目标地址空间中的 `ptr`。调用者必须确保 `ptr` 是有效的、可写的，
///    并且若不跨页，需正确对齐以匹配 `T`，若跨页，则需按字节可寻址。
/// 2. 假设 `token` 正确标识了有效且可访问的页表。
/// 3. 虚拟地址到物理地址的转换可能失败。
/// 4. 跨页写入时逐字节写入可能不满足 `T` 内部字段的严格对齐要求，若 `T` 非平凡类型（POD），可能存在风险。
///
/// # 返回
/// 成功返回 `Ok(())`，失败返回对应的 `PutDataError`。
pub  fn put_data<T:Copy +'static>(token: usize, ptr: *mut T, data: T) -> PutDataRet {
    let data_size = core::mem::size_of::<T>();
    if data_size == 0 {
        return Ok(()); // 零大小类型无需写入
    }

    let start_va = VirtAddr::from(ptr as usize);
    let page_table = PageTable::from_token(token);

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



#[derive(Debug)]
pub enum TranslateError {
    TranslationFailed(VirtAddr),
    DataCrossesPageBoundary,
    UnexpectedEofOrFault,   
    InternalBufferOverflow,  
    LengthOverflow,          
    PermissionDenied(VirtAddr),
    PartialCopy,
    // Add other specific errors if needed, e.g., InsufficientPermissions
}

impl From<TranslateError> for SysErrNo {
    fn from(err: TranslateError) -> Self {
        warn!("TranslateError] err:{:#?},err",err);
        match err {
            TranslateError::TranslationFailed(_) => SysErrNo::ENOMEM,
            TranslateError::DataCrossesPageBoundary => SysErrNo::EFAULT, // Or another appropriate error code
            TranslateError::UnexpectedEofOrFault => SysErrNo::EFAULT,
            TranslateError::InternalBufferOverflow => SysErrNo::EFAULT,
            TranslateError::LengthOverflow => SysErrNo::EFAULT,
            TranslateError::PermissionDenied(_)=> SysErrNo::EACCES,
            TranslateError::PartialCopy => SysErrNo::EFAULT,
        }
    }
}

/// Translate&Copy a ptr[u8] array end with `\0` to a `String` Vec through page table
pub fn translated_str(token: usize, ptr: *const u8) -> String {
    let page_table = PageTable::from_token(token);
    let mut string = String::new();
    let mut va = ptr as usize;
    loop {
        let ch: u8 =
            *(KernelAddr::from(page_table.translate_va(VirtAddr::from(va)).unwrap()).get_mut());
        if ch == 0 {
            break;
        }
        string.push(ch as char);
        va += 1;
    }
    string
}



#[allow(unused)]

/// Translate a ptr[u8] array through page table and return a mutable reference of T
/// 对于没有safe前缀的translated函数 使用前要确保区域必须已分配，而不是懒分配
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> TemplateRet<&'static mut T> {
    let page_table = PageTable::from_token(token);
    let va = ptr as usize;
    match  page_table
        .translate_va(VirtAddr::from(va)){
        Some(pa)=> Ok(pa.get_mut()),
        None => Err(SysErrNo::ENOMEM),
    }
        
}



pub fn fill_str(token: usize, remote_buf: *mut u8, s: &str, max_len: usize) -> Result<(), PutDataError> {
    let bytes = s.as_bytes();
    // 限长：最多写 max_len-1 个字符，最后一个位置放 0
    // Calculate the length of the write operation, ensuring it does not exceed the maximum allowed length.
    // The length is capped at `max_len - 1` to leave space for a null terminator.
    let write_len = core::cmp::min(bytes.len(), max_len - 1);
    for i in 0..write_len {
        // 写每个字符
        unsafe {
            let ptr = remote_buf.add(i);
            put_data(token, ptr, bytes[i])?;
        }
    }
    // 写终止符
    unsafe {
        let term_ptr = remote_buf.add(write_len);
        put_data(token, term_ptr, 0u8)?;
    }
    Ok(())
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
pub fn get_target_ref<'a, T>(token: usize, ptr: *const T) -> Result<&'a T, TranslateError> {
    let va = VirtAddr::from(ptr as usize);
    let size = core::mem::size_of::<T>();
    if size == 0 {
        // For Zero-Sized Types, a dangling pointer is fine as long as it's aligned.
        // However, we still need to ensure the concept of "location" is valid.
        // Translating the VA is a good check.
        // We can return a well-aligned dangling pointer cast to &T.
        let page_table = PageTable::from_token(token);
        page_table.translate_va(va).ok_or(TranslateError::TranslationFailed(va))?;
        // SAFETY: For ZSTs, creating a reference from a dangling but aligned pointer is allowed.
        // return Ok(unsafe { &*(core::ptr::null::<T>() as *const T) }); // Or use ptr::NonNull::dangling()
        return Ok(unsafe { &*core::ptr::NonNull::dangling().as_ptr() });
    }

    let page_table = PageTable::from_token(token);
    let start_pa = page_table.translate_va(va).ok_or(TranslateError::TranslationFailed(va))?;

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



// 假设你的 PageTable, VirtAddr, PhysAddr, TranslateRefError 定义
// 以及 PAGE_SIZE 常量
// use crate::mm::{PageTable, VirtAddr, PhysAddr, TranslateRefError, PAGE_SIZE, page_offset};
// fn page_offset(addr: usize) -> usize { addr % PAGE_SIZE } // 示例

/// 尝试从给定的虚拟地址 `va_start` 开始，获取在同一个物理页内
/// 最多 `max_len` 字节的连续内存区域的引用。
///
/// 返回 `Ok((&'a [u8], usize))`，其中 `&'a [u8]` 是物理连续内存的引用，
/// `usize` 是这个引用实际包含的字节数（可能小于 `max_len`）。
/// 返回 `Err(TranslateRefError)` 如果起始地址翻译失败。
///
/// # Safety
/// - `token` 必须有效。
/// - `va_start` 必须是有效的起始虚拟地址。
/// - 返回的引用的生命周期 `'a` 必须小于等于物理页映射的实际生命周期。
unsafe fn get_target_continuous_bytes_in_page<'a>(
    token: usize,
    va_start: VirtAddr,
    max_len: usize,
) -> Result<(&'a [u8], usize), TranslateError> {
    if max_len == 0 {
        return Ok((&[], 0));
    }

    let page_table = PageTable::from_token(token);
    let pa_start = page_table
        .translate_va(va_start)
        .ok_or(TranslateError::TranslationFailed(va_start))?;

    // TODO: 添加权限检查 (例如，可读性) from page table entry for pa_start

    let start_offset_in_page = va_start.page_offset(); // va_start 在其虚拟页内的偏移
                                                                // 或者 pa_start.as_usize() % PAGE_SIZE
    let bytes_remaining_in_page = PAGE_SIZE - start_offset_in_page;
    let len_to_get = max_len.min(bytes_remaining_in_page);

    if len_to_get == 0 { // 例如 va_start 正好在页的末尾，而 max_len > 0
        return Ok((&[], 0));
    }

    // pa_start.get_ptr::<u8>() 返回 *const u8，指向物理地址
    // 我们确信从这个指针开始的 len_to_get 字节在物理上是连续的，
    // 因为它们都在同一个物理页内。
    let phys_ptr = pa_start.get_ptr::<u8>();
    let byte_slice = slice::from_raw_parts(phys_ptr, len_to_get);

    Ok((byte_slice, len_to_get))
}





/// 安全地从用户空间复制一个 `T` 类型的数组到内核。
/// 处理跨页的用户源数据。
///
/// # Arguments
/// * `token`: 用户地址空间的标识。
/// * `user_src_ptr`: 指向用户空间数组起始位置的指针。
/// * `count`: 数组中元素的数量。
///
/// # Returns
/// `Ok(Vec<T>)` 如果成功复制。Vec 中的 T 保证已初始化。
/// `Err(TranslateRefError)` 如果地址翻译失败或权限不足等。
///
/// # Safety
/// - `token` 必须有效。
/// - `user_src_ptr` 必须是有效的起始虚拟地址，且从该地址开始的 `count * size_of::<T>()` 字节是可读的。
/// - `T` 必须是 `Copy` 或 `Pod` (Plain Old Data) 类型，才能安全地通过字节复制来构造。
///   更通用的版本可能需要 `T: Default` 并逐个构造，或者返回 `Vec<MaybeUninit<T>>`。
///   这里我们简化为返回 `Vec<T>`，并假设 `T` 可以从其字节表示安全地创建。
pub unsafe fn copy_from_user_array<T: Sized + Copy>( // 添加 Copy bound 以安全地 assume_init
    token: usize,
    user_src_ptr: *const T,
    count: usize,
) -> Result<Vec<T>, TranslateError> {
    if count == 0 {
        return Ok(Vec::new());
    }

    let type_size = core::mem::size_of::<T>();
    if type_size == 0 { // 处理 ZST 数组
        // 对于 ZST，我们只需要 count 个默认实例。
        // T: Copy 保证了 ZST 可以被复制。
        // T 必须能被默认构造或零初始化。
        // 如果 T 只是 Copy 而不能默认构造，这里会复杂。
        // 假设 ZST 可以安全地用 `MaybeUninit::zeroed().assume_init()` 创建
        let mut result_vec = Vec::with_capacity(count);
        for _ in 0..count {
            let zst_val: T = MaybeUninit::zeroed().assume_init();
            result_vec.push(zst_val);
        }
        return Ok(result_vec);
    }

    let total_bytes_to_copy = count.checked_mul(type_size).ok_or(TranslateError::LengthOverflow)?; // 防止溢出

    // 创建一个内核缓冲区来接收原始字节
    // 使用 MaybeUninit 来避免初始化 Vec<T> 的元素，因为我们将直接写入字节
    let mut kernel_buffer_uninit: Vec<MaybeUninit<T>> = Vec::with_capacity(count);
    // SAFETY: Vec::with_capacity 后调用 set_len 是安全的，因为我们接下来会初始化所有元素
    //         通过向其底层的 MaybeUninit<u8> 缓冲区写入。
    kernel_buffer_uninit.set_len(count); // 扩展长度以匹配容量

    // 获取指向 MaybeUninit<T> 底层字节缓冲区的可变 slice
    let kernel_dest_byte_slice: &mut [MaybeUninit<u8>] = {
        // SAFETY: MaybeUninit<T> 和 [MaybeUninit<u8>; size_of::<T>()] 布局兼容
        //         MaybeUninit<[T; N]> 和 [MaybeUninit<T>; N] 布局兼容
        //         slice_as_mut_ptr 返回 *mut T，然后转为 *mut MaybeUninit<T>
        let ptr_maybe_uninit_t = kernel_buffer_uninit.as_mut_slice().as_mut_ptr();
        // 将 *mut MaybeUninit<T> 转换为 *mut MaybeUninit<u8>
        let ptr_maybe_uninit_u8 = ptr_maybe_uninit_t as *mut MaybeUninit<u8>;
        slice::from_raw_parts_mut(ptr_maybe_uninit_u8, total_bytes_to_copy)
    };
    // 我们需要一个 &mut [u8] 给 copy_from_user_bytes
    // MaybeUninit::slice_as_mut_ptr 可以得到 *mut T
    // 我们需要 *mut u8
    let kernel_dest_raw_u8_slice: &mut [u8] = {
        // SAFETY: MaybeUninit<T> 与 T 具有相同的布局。
        // 我们将 MaybeUninit<T> 的 slice 视为 u8 的 slice 来填充字节。
        // 这是安全的，因为我们接下来会用 assume_init_vec。
        let ptr_t = kernel_buffer_uninit.as_mut_ptr() as *mut T;
        let ptr_u8 = ptr_t as *mut u8;
        slice::from_raw_parts_mut(ptr_u8, total_bytes_to_copy)
    };


    // 执行字节复制
    copy_from_user_bytes(
        token,
        kernel_dest_raw_u8_slice, // 目标是内核中 Vec<T> 的原始字节区域
        VirtAddr::from(user_src_ptr as usize),
        total_bytes_to_copy,
    )?;

    // 字节已复制到 kernel_buffer_uninit 的位置
    // 现在可以安全地将 Vec<MaybeUninit<T>> 转换为 Vec<T>
    // SAFETY: 我们已经从用户空间将 total_bytes_to_copy 填满了
    //         kernel_buffer_uninit 的底层缓冲区，覆盖了所有 T 类型的实例。
    //         由于 T: Copy，从其字节表示初始化是安全的。
    let result_vec = kernel_buffer_uninit.into_iter().map(|mu| mu.assume_init()).collect();
    // 或者，更直接（但也更 unsafe）的转换如果 Vec<MaybeUninit<T>> 和 Vec<T> 布局保证一致：
    // let result_vec = mem::transmute::<Vec<MaybeUninit<T>>, Vec<T>>(kernel_buffer_uninit);
    // 上面的 map + assume_init 更安全一些。

    Ok(result_vec)
}

/// 安全地从用户空间复制 `len` 字节数据到内核提供的 `kernel_dest_buffer`。
/// 处理跨页的用户源数据。
///
/// # Arguments
/// * `token`: 用户地址空间的标识。
/// * `kernel_dest_buffer`: 内核空间的目标缓冲区，必须至少有 `len` 字节的容量。
/// * `user_src_va_start`: 用户空间的源虚拟地址。
/// * `len`: 要复制的字节数。
///
/// # Returns
/// `Ok(())` 如果成功复制了 `len` 字节。
/// `Err(TranslateRefError)` 如果地址翻译失败或权限不足。
/// `Err(CopyError::BufferTooSmall)` 如果 `kernel_dest_buffer` 不够大 (虽然这里签名是 &mut [u8])
///
/// # Safety
/// - `token` 必须有效。
/// - `user_src_va_start` 必须是有效的起始虚拟地址，且从该地址开始的 `len` 字节是可读的。
/// - `kernel_dest_buffer` 必须指向有效的、可写的内核内存。
pub unsafe fn copy_from_user_bytes(
    token: usize,
    kernel_dest_buffer: &mut [u8], // 目标是内核中的 slice
    mut user_src_va: VirtAddr,
    len_to_copy: usize,
) -> Result<(), TranslateError> { // 可以定义一个更通用的 CopyError
    if len_to_copy > kernel_dest_buffer.len() {
        // return Err(CopyError::BufferTooSmall); // 或者 panic，或调整 API
        // 对于 &mut [u8]，我们假设调用者保证了它足够大。
        // 但最好还是检查一下。
        if cfg!(debug_assertions) {
            panic!("copy_from_user_bytes: kernel_dest_buffer too small");
        }
        // 或者返回一个错误，这里暂时简化为不处理这个特定错误
    }

    let mut bytes_copied: usize = 0;
    let mut dest_slice_offset: usize = 0;

    while bytes_copied < len_to_copy {
        let remaining_len = len_to_copy - bytes_copied;
        // 尝试从当前物理页获取尽可能多的字节
        match get_target_continuous_bytes_in_page(token, user_src_va, remaining_len) {
            Ok((user_page_slice, len_in_page)) => {
                if len_in_page == 0 {
                    // 这不应该发生，除非 remaining_len 也是0，或者 va 在页末尾且 remaining_len > 0
                    // 如果 va 在页末尾，get_target_continuous_bytes_in_page 会返回 Ok((&[],0))
                    // 但下一次循环 user_src_va 会增加，进入下一页。
                    // 如果 len_in_page 为0但 remaining_len > 0，意味着无法读取更多数据，可能是 EFAULT。
                    // 不过 get_target_continuous_bytes_in_page 本身会处理 TranslationFailed。
                    // 这里表示一个逻辑问题或无法满足的读取。
                    return Err(TranslateError::UnexpectedEofOrFault); // 需要定义这个错误
                }

                // 确定实际要复制的字节数（不能超过目标缓冲区的剩余容量）
                let copy_now_len = len_in_page.min(kernel_dest_buffer.len() - dest_slice_offset);
                if copy_now_len == 0 && remaining_len > 0 {
                    // 内核目标缓冲区已满，但还有用户数据要读
                    if cfg!(debug_assertions) {
                        panic!("copy_from_user_bytes: kernel_dest_buffer became full unexpectedly");
                    }
                    return Err(TranslateError::InternalBufferOverflow); // 需要定义
                }


                kernel_dest_buffer[dest_slice_offset..dest_slice_offset + copy_now_len]
                    .copy_from_slice(&user_page_slice[0..copy_now_len]);

                bytes_copied += copy_now_len;
                user_src_va = user_src_va + copy_now_len;
                dest_slice_offset += copy_now_len;
            }
            Err(e) => return Err(e), // 传递翻译错误
        }
    }
    Ok(())
}






/// 尝试从给定的虚拟地址 `va_start` 开始，获取在同一个物理页内
/// 最多 `max_len` 字节的连续内存区域的可变引用。
///
/// # Safety
/// - `token` 必须有效。
/// - `va_start` 必须是有效的起始虚拟地址。
/// - 返回的引用的生命周期 `'a` 必须小于等于物理页映射的实际生命周期。
/// - 调用者必须确保对这块内存的独占可变访问，以避免数据竞争。
unsafe fn get_target_continuous_writable_bytes_in_page<'a>(
    token: usize,
    va_start: VirtAddr,
    max_len: usize,
) -> Result<(&'a mut [u8], usize), TranslateError> {
    if max_len == 0 {
        return Ok((&mut [], 0));
    }

    let page_table = PageTable::from_token(token);
    // 现在需要检查可写权限
    let pa_start = page_table
        .translate_va_with_perm(va_start, true /* require_writable */)?;
      
       


    let start_offset_in_page = va_start.page_offset();
    let bytes_remaining_in_page = PAGE_SIZE - start_offset_in_page;
    let len_to_get = max_len.min(bytes_remaining_in_page);

    if len_to_get == 0 {
        return Ok((&mut [], 0));
    }

    // pa_start.get_mut_ptr::<u8>() 返回 *mut u8，指向物理地址
    let phys_ptr_mut = pa_start.get_mut::<u8>();
    let byte_slice_mut = slice::from_raw_parts_mut(phys_ptr_mut, len_to_get);

    Ok((byte_slice_mut, len_to_get))
}


/// 安全地从内核缓冲区 `kernel_src_buffer` 复制数据到用户空间 `user_dest_va`。
/// 处理用户空间目标地址可能跨页的情况。
///
/// # Arguments
/// * `token`: 用户地址空间的标识。
/// * `user_dest_va_start`: 用户空间的目标虚拟地址的起始。
/// * `kernel_src_buffer`: 内核空间的源数据切片。
/// * `len_to_copy`: 要复制的字节数。如果 `len_to_copy` 大于 `kernel_src_buffer.len()`，
///                  则只会复制 `kernel_src_buffer.len()` 字节。
///                  (或者可以设计为返回错误或 panic)。
///                  这里我们假设 `len_to_copy <= kernel_src_buffer.len()` 由调用者保证，
///                  或者我们只复制两者中较小的长度。
///
/// # Returns
/// `Ok(usize)`: 成功复制的字节数。
/// `Err(TranslateRefError)`: 如果地址翻译、权限或复制过程中发生错误。
///
/// # Safety
/// - `token` 必须有效。
/// - `user_dest_va_start` 必须是有效的起始虚拟地址，且从该地址开始的 `len_to_copy` 字节
///   在用户空间中是有效的、可写的内存区域。
/// - `kernel_src_buffer` 必须指向有效的、可读的内核内存。
pub unsafe fn copy_to_user_bytes(
    token: usize,
    mut user_dest_va: VirtAddr,      // 用户空间目标虚拟地址
    kernel_src_buffer: &[u8],       // 内核源数据
    // len_to_copy: usize,          // 要复制的字节数，现在由 kernel_src_buffer.len() 决定
) -> Result<usize, TranslateError> { // 返回实际复制的字节数
    let len_to_copy = kernel_src_buffer.len();
    if len_to_copy == 0 {
        return Ok(0);
    }

    let mut bytes_copied: usize = 0;
    let mut src_slice_offset: usize = 0; // 内核源缓冲区的偏移

    while bytes_copied < len_to_copy {
        let remaining_len_to_copy_from_kernel = len_to_copy - bytes_copied;

        // 尝试获取用户空间当前物理页的可写字节区域
        // get_target_continuous_writable_bytes_in_page 会限制长度到页尾
        match get_target_continuous_writable_bytes_in_page(
            token,
            user_dest_va,
            remaining_len_to_copy_from_kernel, // 最多复制剩余的字节
        ) {
            Ok((user_page_mut_slice, len_writable_in_page)) => {
                if len_writable_in_page == 0 {
                    // 无法在用户页获得可写空间，但仍有数据要复制。
                    // 这可能意味着 user_dest_va 在页末尾，或者是一个错误。
                    // get_target_continuous_writable_bytes_in_page 返回 Ok(&mut [], 0) 如果 va 在页尾且 max_len > 0
                    // 如果 remaining_len_to_copy_from_kernel > 0，这不应该发生除非页本身就有问题。
                    // 通常表示 EFAULT 或类似的页问题。
                    // 如果是因为 va 在页末尾，下一次迭代 user_dest_va 会增加。
                    // 但如果 max_len (即 remaining_len...) 也是0，则不会到这里。
                    // 为了安全，如果 len_writable_in_page 是0但还有数据要写，则认为是错误。
                    if remaining_len_to_copy_from_kernel > 0 {
                        // log::warn!("copy_to_user: Got 0 writable bytes in user page {:?} but still have {} bytes to copy.", user_dest_va, remaining_len_to_copy_from_kernel);
                        return Err(TranslateError::UnexpectedEofOrFault); // 或者 EFAULT
                    } else {
                        break; // 没有剩余数据要复制了
                    }
                }

                // 从内核源缓冲区复制到用户页的可写切片
                // 只复制实际能在当前用户页写入的，并且不超过内核缓冲区剩余的
                let actual_bytes_this_pass = len_writable_in_page; // 已经被 min(remaining_len, bytes_in_page) 限制

                user_page_mut_slice[0..actual_bytes_this_pass]
                    .copy_from_slice(
                        &kernel_src_buffer[src_slice_offset .. src_slice_offset + actual_bytes_this_pass]
                    );

                bytes_copied += actual_bytes_this_pass;
                user_dest_va = user_dest_va + actual_bytes_this_pass;
                src_slice_offset += actual_bytes_this_pass;
            }
            Err(e) => return Err(e), // 传递翻译或权限错误
        }
    }
    Ok(bytes_copied)
}



/// Translate&Copy a ptr[u8] array with LENGTH len to a mutable u8 Vec through page table
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
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

/// 安全地从内核缓冲区 `kernel_src_buffer` 将其**全部内容**复制到用户空间 `user_dest_va`。
/// 如果未能复制所有字节，则返回错误。
///
/// # Arguments
/// * `token`: 用户地址空间的标识。
/// * `user_dest_va_start`: 用户空间的目标虚拟地址的起始。
/// * `kernel_src_buffer`: 内核空间的源数据切片。其全部内容将被尝试复制。
///
/// # Returns
/// `Ok(())` 如果成功复制了 `kernel_src_buffer.len()` 字节。
/// `Err(TranslateRefError)` 如果地址翻译、权限或复制过程中发生错误，导致未能复制所有字节。
///
/// # Safety
/// - `token` 必须有效。
/// - `user_dest_va_start` 必须是有效的起始虚拟地址，且从该地址开始的
///   `kernel_src_buffer.len()` 字节在用户空间中是有效的、可写的内存区域。
/// - `kernel_src_buffer` 必须指向有效的、可读的内核内存。
pub unsafe fn copy_to_user_bytes_exact(
    token: usize,
    user_dest_va: VirtAddr,
    kernel_src_buffer: &[u8],
) -> Result<(), TranslateError> {
    let len_to_copy = kernel_src_buffer.len();
    if len_to_copy == 0 {
        return Ok(()); // 复制0字节总是成功的
    }

    // 调用你现有的 copy_to_user_bytes
    match copy_to_user_bytes(token, user_dest_va, kernel_src_buffer) {
        Ok(bytes_copied) => {
            if bytes_copied == len_to_copy {
                Ok(()) // 所有字节都成功复制了
            } else {
                // 未能复制所有请求的字节，这对于 "exact" 版本来说是错误
                // log::warn!(
                //     "copy_to_user_bytes_exact: Expected to copy {} bytes, but only copied {}. User VA: {:?}",
                //     len_to_copy, bytes_copied, user_dest_va
                // );
                // 可能的原因是用户提供的缓冲区部分无效，或者 get_target_continuous_writable_bytes_in_page
                // 返回的 len_writable_in_page 累加起来不足 len_to_copy。
                // TranslateRefError::UnexpectedEofOrFault 比较适合这种情况。
                Err(TranslateError::PartialCopy)
            }
        }
        Err(e) => {
            // 如果 copy_to_user_bytes 本身返回了错误（例如翻译失败、权限问题），
            // 直接传递这个错误。
            Err(e)
        }
    }
}


pub unsafe fn copy_from_user_exact<T: Copy>(token: usize, user_src: *const T) -> Result<T, TranslateError> {
    let mut kernel_val = core::mem::MaybeUninit::<T>::uninit();
    copy_from_user_bytes(
        token,
        core::slice::from_raw_parts_mut(kernel_val.as_mut_ptr() as *mut u8, core::mem::size_of::<T>()),
        VirtAddr::from(user_src as usize),
        core::mem::size_of::<T>()
    )?; 
    Ok(kernel_val.assume_init())
}


