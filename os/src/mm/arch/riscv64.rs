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
        const COW=1<<8;
        const W_BACKUP= 1<<9;
        
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
    pub fn is_cow(&self)->bool{
        PTEFlags::from_bits_truncate(self.bits).contains(PTEFlags::R | PTEFlags::W)
    }
       // 判断是否设置了 W_BACKUP
    pub fn is_back_w(&self) -> bool {
        self.flags().contains(PTEFlags::W_BACKUP)
    }
    pub fn set_cow(&mut self) {
        let mut flags = self.flags();
        if flags.contains(PTEFlags::W) {
            flags.insert(PTEFlags::W_BACKUP);
        }
        flags.remove(PTEFlags::W);
        flags.insert(PTEFlags::COW);
        self.set_raw_flags(flags);
    }
        /// 解除 COW 状态，并根据 W_BACKUP 恢复 W 权限
    pub fn un_cow(&mut self) {
            let mut flags = self.flags();
    
            if flags.contains(PTEFlags::W_BACKUP) {
                flags.insert(PTEFlags::W);
            }
    
            flags.remove(PTEFlags::COW);
            flags.remove(PTEFlags::W_BACKUP);
    
            self.set_raw_flags(flags);
        }
    // 内部使用：安全更新 flags 保留物理地址
    fn set_raw_flags(&mut self, flags: PTEFlags) {
        self.bits = (self.bits & Self::PHYS_ADDR_MASK) | flags.bits();
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