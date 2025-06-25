use core::fmt;
use bit_field::BitField;
use crate::mm::{PhysPageNum, MapPermission};
use crate::config::PAGE_SIZE_BITS;

const PALEN: usize = 48;

bitflags::bitflags! {
    pub struct PTEFlags: usize {
        const V = 1 << 0;
        const D = 1 << 1;
        const PLVL = 1 << 2;
        const PLVH = 1 << 3;
        const MATL = 1 << 4;
        const MATH = 1 << 5;
        const G = 1 << 6;
        const P = 1 << 7;
        const W = 1 << 8;
        const NR = 1 << 61;
        const NX = 1 << 62;
        const RPLV = 1 << 63;
        
        // COW 相关标志位 - 使用空闲的位
        const COW = 1 << 9;      // Copy-On-Write 标志
        const W_BACKUP = 1 << 10; // 写权限备份
    }
}

impl PTEFlags {
    fn default() -> Self {
        PTEFlags::V | PTEFlags::MATL | PTEFlags::P | PTEFlags::W
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct PageTableEntry {
    pub bits: usize,
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "PageTableEntry RPLV:{},NX:{},NR:{},PPN:{:#x},W:{},P:{},G:{},MAT:{},PLV:{},D:{},V:{}",
            (self.bits >> 63) & 1,
            (self.bits >> 62) & 1,
            (self.bits >> 61) & 1,
            (self.bits >> 14) & ((1usize << (PALEN - 14)) - 1),
            (self.bits >> 8) & 1,
            (self.bits >> 7) & 1,
            (self.bits >> 6) & 1,
            (self.bits >> 4) & 3,
            (self.bits >> 2) & 3,
            (self.bits >> 1) & 1,
            self.bits & 1
        )
    }
}

impl PageTableEntry {
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        let mut bits = flags.bits();
        bits |= (ppn.0 & ((1usize << (PALEN - 14)) - 1)) << 14;
        PageTableEntry { bits }
    }

    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }

    pub fn ppn(&self) -> PhysPageNum {
        ((self.bits >> 14) & ((1usize << (PALEN - 14)) - 1)).into()
    }

    pub fn directory_ppn(&self) -> PhysPageNum {
        (self.bits >> PAGE_SIZE_BITS).into()
    }

    pub fn flags(&self) -> PTEFlags {
        let mut bits = self.bits;
        bits &= (1usize << 14) - 1; // 保留低14位的标志位
        bits |= self.bits & (7usize << 61); // 保留高3位的标志位
        bits |= self.bits & (3usize << 9);  // 保留 COW 相关位
        PTEFlags::from_bits_truncate(bits)
    }

    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }

    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }

    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::NR) == PTEFlags::empty()
    }

    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::NX) == PTEFlags::empty()
    }

    pub fn set_dirty(&mut self) {
        self.bits |= PTEFlags::D.bits();
    }

    pub fn is_zero(&self) -> bool {
        self.bits == 0
    }

    // COW 相关方法
    pub fn set_cow(&mut self) {
        let mut flags = self.flags();
        
        // 如果当前可写，则备份写权限并清除写权限
        if flags.contains(PTEFlags::W) {
            flags.insert(PTEFlags::W_BACKUP);
            flags.remove(PTEFlags::W);
        }
        
        // 设置 COW 标志
        flags.insert(PTEFlags::COW);
        
        // 更新页表项
        self.set_flags(flags);
    }

    pub fn is_cow(&self) -> bool {
        self.flags().contains(PTEFlags::COW)
    }

    pub fn clear_cow(&mut self) {
        let mut flags = self.flags();
        
        // 清除 COW 标志
        flags.remove(PTEFlags::COW);
        
        // 如果有备份的写权限，则恢复
        if flags.contains(PTEFlags::W_BACKUP) {
            flags.insert(PTEFlags::W);
            flags.remove(PTEFlags::W_BACKUP);
        }
        
        // 更新页表项
        self.set_flags(flags);
    }

    pub fn set_flags(&mut self, flags: PTEFlags) {
        // 保留 PPN 部分，更新标志位
        let ppn_bits = self.bits & !((1usize << 14) - 1) & !((7usize << 61) | (3usize << 9));
        self.bits = ppn_bits | flags.bits();
    }
    
    pub fn set_ppn(&mut self, paddr: crate::mm::PhysAddr) {
        let ppn = crate::mm::PhysPageNum::from(paddr.0 >> PAGE_SIZE_BITS);
        let flags_bits = self.bits & ((1usize << 14) - 1) | (self.bits & (7usize << 61)) | (self.bits & (3usize << 9));
        self.bits = flags_bits | ((ppn.0 & ((1usize << (PALEN - 14)) - 1)) << 14);
    }
}

impl From<MapPermission> for PTEFlags {
    fn from(f: MapPermission) -> Self {
        if f.is_empty() {
            return Self::empty();
        }
        let mut ret = Self::V | Self::P;
        
        if !f.contains(MapPermission::R) {
            ret |= Self::NR;
        }
        if f.contains(MapPermission::W) {
            ret |= Self::W;
        }
        if !f.contains(MapPermission::X) {
            ret |= Self::NX;
        }
        if f.contains(MapPermission::U) {
            ret |= Self::PLVL | Self::PLVH;
        }
        
        ret
    }
}

