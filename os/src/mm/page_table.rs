//! Implementation of [`PageTableEntry`] and [`PageTable`].



use super::{KernelAddr, KERNEL_PAGE_TABLE_PPN};
use super::{frame_alloc, FrameTracker, PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;
use crate::config;
use crate::utils::error::{ SysErrNo, TemplateRet};
use crate::utils::is_aligned_to;
bitflags! {
    /// page table entry flags
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
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
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
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
}

/// page table structure
pub struct PageTable {

    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

/// Assume that it won't oom when creating/mapping.
impl PageTable {
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
    /// get the token from the page table
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
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


/// An abstraction over a buffer passed from user space to kernel space
pub struct UserBuffer {
    /// A list of buffers
    pub buffers: Vec<&'static mut [u8]>,
}

impl UserBuffer {
    /// Constuct UserBuffer
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }
    /// Get the length of the buffer
    pub fn len(&self) -> usize {
        let mut total: usize = 0;
        for b in self.buffers.iter() {
            total += b.len();
        }
        total
    }
}

impl IntoIterator for UserBuffer {
    type Item = *mut u8;
    type IntoIter = UserBufferIterator;
    fn into_iter(self) -> Self::IntoIter {
        UserBufferIterator {
            buffers: self.buffers,
            current_buffer: 0,
            current_idx: 0,
        }
    }
}

/// An iterator over a UserBuffer
pub struct UserBufferIterator {
    buffers: Vec<&'static mut [u8]>,
    current_buffer: usize,
    current_idx: usize,
}

impl Iterator for UserBufferIterator {
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






// /// 将数据 `data` 写入 `token` 地址空间 `ptr` 处，
// /// 其中虚拟地址 `ptr` 解析得到的物理地址可以跨页
// pub fn put_data<T: 'static>(token: usize, ptr: *mut T, data: T)->GeneralRet {
//     let page_table = PageTable::from_token(token);
//     let mut va = VirtAddr::from(ptr as usize);
//     let pa = page_table.translate_va(va).unwrap();
//     let size = core::mem::size_of::<T>();
//     // 若数据跨页，则转换成字节数据写入
//     if (pa + size - 1).floor() != pa.floor() {
//         let bytes =
//             unsafe { core::slice::from_raw_parts(&data as *const _ as usize as *const u8, size) };
//         for i in 0..size {
//             *(page_table.translate_va(va).unwrap().get_mut()) = bytes[i];
//             va = va + 1;
//         }
//     } else {
//         *translated_refmut(token, ptr)? = data;
//     };
//     Ok(())
// }


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

    // 检查数据是否能完全放入从 start_pa 开始的单页内
    // （start_pa.as_usize() / PAGE_SIZE） == ((start_pa.as_usize() + data_size - 1) / PAGE_SIZE)
    // 更严谨的方式：
    // start_pa.floor() == (start_pa + data_size - 1).floor()
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
        
        
        
        // 如果没有 `translated_refmut`，则需要自行写物理地址
        // 检查从 start_va 到 start_va + data_size - 1 所有字节均映射
        // for offset in 0..data_size {
        //     if page_table.translate_va(start_va + offset).is_none() {
        //         return Err(PutDataError::TranslationFailed(start_va + offset));
        //     }
        // }
     
      
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

// 辅助函数：获取目标地址空间中某虚拟地址对应的可变引用。
// 这是 `translated_refmut` 可能的更完整版本，针对单页、对齐且权限已校验的写入。
// 该版本仍不安全，且有假设。
#[allow(dead_code)]
pub fn get_target_ref_mut<'a, T: 'static>(
    token: usize,
    ptr: *mut T,
) -> Result<&'a mut T, PutDataError> {
    // 把传入的裸指针转成虚拟地址类型
    let va = VirtAddr::from(ptr as usize);
    // 计算类型 T 的大小
    let size = core::mem::size_of::<T>();
    // 根据传入的 token 创建页表对象
    let page_table = PageTable::from_token(token);

    // 1. Check alignment of the virtual address if T has alignment requirements > 1
    // 1. 如果类型 T 的对齐要求大于 1，检查虚拟地址的对齐性
    if core::mem::align_of::<T>() > 1 && !is_aligned_to(va.0,core::mem::align_of::<T>()) {
        return Err(PutDataError::UnalignedAccess); // Or handle via unaligned write
    }

    // 2. Translate start and end addresses to ensure contiguous mapping (simplified check)
    // 2. 翻译起始和结束虚拟地址，确保它们映射到连续的物理内存（简化检查）
    let start_pa = page_table.translate_va(va).ok_or(PutDataError::TranslationFailed(va))?;
    if size > 0 {
        // 计算结束虚拟地址
        let end_va = va + (size - 1);
        let end_pa = page_table.translate_va(end_va).ok_or(PutDataError::TranslationFailed(end_va))?;
        // 简单检查：如果起始和结束地址映射到同一物理页，且大小小于页面大小，则假设内存连续。
        // 这里更严格的检查需要验证所有跨越的虚拟页，并确认物理地址连续。
        if start_pa.floor() != end_pa.floor() {
            // 如果跨越了物理页边界，当前函数不支持这种情况。
            // 也可以当作错误处理。
            return Err(PutDataError::TranslationFailed(va)); // 或者更具体的错误
        }
        // 如果跨越虚拟页但物理连续，做更好的物理连续性检查：
        if end_pa.0 != start_pa.0 + size - 1 {
            // 物理内存不连续，可能是虚拟页映射到不连续的物理帧
            // return Err(PutDataError::NonContiguousPhysicalMemory);
        }
    }

   
    // 3. 待完成：检查页表权限（例如写权限）

    // SAFETY: Caller ensures ptr is valid. We've done basic translation checks.
    // 安全性说明：调用者确保 ptr 有效，这里已做了基本的地址转换检查。
    Ok(&mut *(start_pa.get_mut::<T>()))
}

#[derive(Debug)]
pub enum TranslateRefError {
    TranslationFailed(VirtAddr),
    DataCrossesPageBoundary,
    // Add other specific errors if needed, e.g., InsufficientPermissions
}

impl From<TranslateRefError> for SysErrNo {
    fn from(err: TranslateRefError) -> Self {
        match err {
            TranslateRefError::TranslationFailed(_) => SysErrNo::ENOMEM,
            TranslateRefError::DataCrossesPageBoundary => SysErrNo::EFAULT, // Or another appropriate error code
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
/// 不支持跨页
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> TemplateRet<&'static mut T> {
    let page_table = PageTable::from_token(token);
    let va = ptr as usize;
    match  page_table
        .translate_va(VirtAddr::from(va)){
        Some(pa)=> Ok(pa.get_mut()),
        None => Err(SysErrNo::ENOMEM),
    }
        
}


/// Translates a virtual address `ptr` from the address space identified by `token`
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
pub fn translated_ref<'a, T>(token: usize, ptr: *const T) -> Result<&'a T, TranslateRefError> {
    let va = VirtAddr::from(ptr as usize);
    let size = core::mem::size_of::<T>();
    if size == 0 {
        // For Zero-Sized Types, a dangling pointer is fine as long as it's aligned.
        // However, we still need to ensure the concept of "location" is valid.
        // Translating the VA is a good check.
        // We can return a well-aligned dangling pointer cast to &T.
        let page_table = PageTable::from_token(token);
        page_table.translate_va(va).ok_or(TranslateRefError::TranslationFailed(va))?;
        // SAFETY: For ZSTs, creating a reference from a dangling but aligned pointer is allowed.
        // return Ok(unsafe { &*(core::ptr::null::<T>() as *const T) }); // Or use ptr::NonNull::dangling()
        return Ok(unsafe { &*core::ptr::NonNull::dangling().as_ptr() });
    }

    let page_table = PageTable::from_token(token);
    let start_pa = page_table.translate_va(va).ok_or(TranslateRefError::TranslationFailed(va))?;

    // Check for cross-page boundary for the physical address
    // (va.as_usize() / PAGE_SIZE) != ((va.as_usize() + size - 1) / PAGE_SIZE)
    // Better: start_pa.floor() != (start_pa + size - 1).floor()
    if size > 0 && start_pa.floor() != (start_pa + (size - 1)).floor() {
        return Err(TranslateRefError::DataCrossesPageBoundary);
    }

    // TODO: Add permission checks (e.g., readability) from page table entry if possible.

    // SAFETY: Caller ensures validity. We've checked translation and single-page constraint.
    // The lifetime 'a should be tied to the validity of the mapping.
    // Using 'static is dangerous unless the mapping is truly static.
    Ok(unsafe { &*start_pa.get_ptr::<T>() }) // Assuming PhysAddr::as_ptr() returns *const T
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
