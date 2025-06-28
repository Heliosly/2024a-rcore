use core::arch::asm;
use core::fmt;
use bit_field::BitField;
use crate::mm::{MapPermission, PhysAddr, PhysPageNum};
use crate::config::PAGE_SIZE_BITS;

const PALEN: usize = 48;
    /// The size of the page for this platform.
    pub const PAGE_SIZE: usize = 0x1000;
    pub const PAGE_LEVEL: usize = 3;
    pub const PTE_NUM_IN_PAGE: usize = 0x200;

    macro_rules! bit {
        ($x: expr) => {
            (1 << $x)
        };
    }
bitflags::bitflags! {
    pub struct PTEFlags: usize {
        /// Page Valid
        const V = bit!(0);
        /// Dirty, The page has been writed.
        const D = bit!(1);

        const PLV_USER = 0b11 << 2;

        const MAT_NOCACHE = 0b01 << 4;

        /// Designates a global mapping OR Whether the page is huge page.
        const GH = bit!(6);

        /// Page is existing.
        const P = bit!(7);
        /// Page is writeable.
        const W = bit!(8);
        /// Is a Global Page if using huge page(GH bit).
        const G = bit!(10);
        /// Page is not readable.
        const NR = bit!(11);
        /// Page is not executable.
        /// FIXME: Is it just for a huge page?
        /// Linux related url: https://github.com/torvalds/linux/blob/master/arch/loongarch/include/asm/pgtable-bits.h
        const NX = bit!(12);
        /// Whether the privilege Level is restricted. When RPLV is 0, the PTE
        /// can be accessed by any program with privilege Level highter than PLV.
        const RPLV = bit!(63);
        // COW 相关标志位 - 使用空闲的位
        const COW = 1 << 49;      // Copy-On-Write 标志
        const W_BACKUP = 1 << 48; // 写权限备份
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
            (self.bits >> PAGE_SIZE_BITS) & ((1usize << (PALEN - PAGE_SIZE_BITS)) - 1),
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
    #[inline]
    pub fn new_table(paddr: PhysAddr) -> Self {
        assert!(paddr.0%PAGE_SIZE==0);
        Self{bits: paddr.0 }
    }

    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        let mut bits = 0usize;
        bits.set_bits(PAGE_SIZE_BITS..PALEN, ppn.0); //采用16kb大小的页
        bits = bits | flags.bits;
        PageTableEntry { bits }
    }
    // 空页表项
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    // 返回物理页号---页表项
    pub fn ppn(&self) -> PhysPageNum {
        self.bits.get_bits(PAGE_SIZE_BITS..PALEN).into()
    }
    pub fn address(&self)->PhysAddr{
        PhysAddr::from((self.bits) & 0xffff_ffff_f000)
    }
    // 返回物理页号---页目录项
    // 在一级和二级页目录表中目录项存放的是只有下一级的基地址
    pub fn directory_ppn(&self) -> PhysPageNum {
        (self.bits >> PAGE_SIZE_BITS).into()
    }
    // 返回标志位
    pub fn flags(&self) -> PTEFlags {
        //这里只需要标志位，需要把非标志位的位置清零
        let mut bits = self.bits;
        bits.set_bits(PAGE_SIZE_BITS..PALEN, 0);
        PTEFlags::from_bits(bits).unwrap()
    }
    // 有效位
    pub fn is_valid(&self) -> bool {
        self.bits !=0
    }
    // 是否可写
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    // 是否可读
    pub fn readable(&self) -> bool {
        !((self.flags() & PTEFlags::NR) != PTEFlags::empty())
    }
    // 是否可执行
    pub fn executable(&self) -> bool {
        !((self.flags() & PTEFlags::NX) != PTEFlags::empty())
    }
    //设置脏位
    pub fn set_dirty(&mut self) {
        self.bits.set_bit(1, true);
    }
    // 用于判断存放的页目录项是否为0
    // 由于页目录项只保存下一级目录的基地址
    // 因此判断是否是有效的就只需判断是否为0即可
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
        let ppn_bits = self.bits & !((1usize << PAGE_SIZE_BITS) - 1) & !((7usize << 61) | (3usize << 9));
        self.bits = ppn_bits | flags.bits();
    }
    
    pub fn set_ppn(&mut self, paddr: crate::mm::PhysAddr) {
        let ppn = crate::mm::PhysPageNum::from(paddr.0 >> PAGE_SIZE_BITS);
        let flags_bits = self.bits & ((1usize << PAGE_SIZE_BITS) - 1) | (self.bits & (7usize << 61)) | (self.bits & (3usize << 9));
        self.bits = flags_bits | ((ppn.raw()& ((1usize << (PALEN - PAGE_SIZE_BITS)) - 1)) << PAGE_SIZE_BITS);
    }
}

impl From<MapPermission> for PTEFlags {
    fn from(value: MapPermission) -> Self {
        let mut flags = PTEFlags::V|PTEFlags::P;
        if value.contains(MapPermission::W) {
            flags |= PTEFlags::W | PTEFlags::D;
        }

        // if !value.contains(MapPermission::X) {
        //     flags |= PTEFlags::NX;
        // }

        if value.contains(MapPermission::U) {
            flags |= PTEFlags::PLV_USER;
        }
        flags
    }
}

impl From<PTEFlags> for MapPermission {
    fn from(val: PTEFlags) -> Self {
        let mut flags = MapPermission::empty();
        if val.contains(PTEFlags::W) {
            flags |= MapPermission::W;
        }

        if val.contains(PTEFlags::D) {
            flags |= MapPermission::D;
        }

        // if !self.contains(PTEFlags::NX) {
        //     flags |= MapPermission::X;
        // }

        if val.contains(PTEFlags::PLV_USER) {
            flags |= MapPermission::U;
        }
        flags
    }
}