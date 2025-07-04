//! Implementation of physical and virtual address and page number.
//use alloc::fmt::format;

use super:: PageTableEntry;
use crate::config::{PAGE_SIZE, PAGE_SIZE_BITS,KERNEL_DIRECT_OFFSET};
use core::{fmt::{self, Debug, Formatter}, ops::{Add, Sub}, panic};

const PA_WIDTH_SV39: usize = 56;
const VA_WIDTH_SV39: usize = 39;
const PPN_WIDTH_SV39: usize = PA_WIDTH_SV39 - PAGE_SIZE_BITS;
const VPN_WIDTH_SV39: usize = VA_WIDTH_SV39 - PAGE_SIZE_BITS;
/// Definitions
#[repr(C)]
#[derive(Copy,Clone,Ord,PartialOrd,Eq,PartialEq)]
///Kernel Address
pub struct KernelAddr(pub usize);
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
///Phys Address
pub struct PhysAddr(pub usize);

/// Virtual Address
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
///virtual address
pub struct VirtAddr(pub usize);

/// Physical Page Number PPN
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
///phiscal page number
pub struct PhysPageNum(pub usize);
impl PhysPageNum  {
    pub fn raw(&self)->usize{
        self.0
     }
}
/// Virtual Page Number VPN
#[repr(C)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtPageNum(pub usize);
impl VirtPageNum  {
   pub fn raw(&self)->usize{
       self.0
    }
    /// Get the index into the n-th level page table for this page number.
    ///
    /// For level n, take bits [9*n .. 9*n+9) of the page number.
    #[inline]
    pub fn pn_index(&self, n: usize) -> usize {
        // 每级 9 位索引
        (self.raw() >> (9 * n)) & 0x1ff
    }

    /// Get the offset within the page number up to level n.
    ///
    /// That is, take the low 9*n bits of the page number.
    #[inline]
    pub fn pn_offset(&self, n: usize) -> usize {
        let mask = (1 << (9 * n)) - 1;
        self.raw() & mask
    }
}
/// Debugging

impl Debug for VirtAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("VA:{:#x}", self.0))
    }
}
impl Debug for VirtPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("VPN:{:#x}", self.0))
    }
}
impl Debug for PhysAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("PA:{:#x}", self.0))
    }
}
impl Debug for PhysPageNum {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("PPN:{:#x}", self.0))
    }
}
impl Debug for KernelAddr{
    fn fmt(&self,f:&mut Formatter<'_>)->fmt::Result{
        f.write_fmt(format_args!("KA:{:#x}",self.0))
    }
}
/// T: {PhysAddr, VirtAddr, PhysPageNum, VirtPageNum}
/// T -> usize: T.0
/// usize -> T: usize.into()
impl From <usize> for KernelAddr{
    fn from(v: usize) -> Self {
        Self(v )
    }
    
}
impl From<KernelAddr> for PhysPageNum {
    fn from(ka: KernelAddr) -> Self {
        let pa = PhysAddr::from(ka);
        pa.floor()
    }
}
impl From<PhysAddr> for KernelAddr {
    fn from(pa: PhysAddr) -> Self {
     assert!(pa.0!=0);
     Self(pa.0 + (KERNEL_DIRECT_OFFSET ))
    }
}

impl From<KernelAddr> for PhysAddr {
    fn from(ka: KernelAddr) -> Self {
        
        Self(ka.0 - (KERNEL_DIRECT_OFFSET))
    }
}

impl From<KernelAddr> for VirtAddr {
    fn from(ka: KernelAddr) -> Self {
        Self(ka.0)
    }
}

impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        // Self(v & ((1 << PA_WIDTH_SV39) - 1))

        #[cfg(target_arch = "riscv64")]
        {let tmp = (v as isize >> PA_WIDTH_SV39) as isize;

        assert!(tmp == 0 || tmp == -1);
        }
        Self(v)
    }
}
impl From<usize> for PhysPageNum {
    fn from(v: usize) -> Self {
        // Self(v & ((1 << PPN_WIDTH_SV39) - 1))
        #[cfg(target_arch = "riscv64")]
        let tmp = (v as isize >> PPN_WIDTH_SV39) as isize;
        #[cfg(target_arch = "riscv64")]
        assert!(tmp == 0 || tmp == -1);
        Self(v)
    }
}
/// 拓展虚拟地址到512GB
impl From<usize> for VirtAddr {
    fn from(v: usize) -> Self {
        // Self(v & ((1 << VA_WIDTH_SV39) - 1))
        #[cfg(target_arch = "riscv64")]
        let tmp = (v as isize >> VA_WIDTH_SV39) as isize;
        #[cfg(target_arch = "riscv64")]
        assert!(tmp == 0 || tmp == -1, "invalid va: {:#x}", v);
        Self(v)
    }
}
impl From<usize> for VirtPageNum {
    fn from(v: usize) -> Self {
        // Self(v & ((1 << VPN_WIDTH_SV39) - 1))
        #[cfg(target_arch = "riscv64")]
        let tmp = v >> (VPN_WIDTH_SV39 - 1);

        #[cfg(target_arch = "riscv64")]
        assert!(tmp == 0 || tmp == (1 << (52 - VPN_WIDTH_SV39 + 1)) - 1);
        Self(v)
    }
}
impl From<PhysAddr> for usize {
    fn from(v: PhysAddr) -> Self {
        v.0
    }
}
impl From<PhysPageNum> for usize {
    fn from(v: PhysPageNum) -> Self {
        v.0
    }
}
impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        if v.0 >= (1 << (VA_WIDTH_SV39 - 1)) {
            v.0 | (!((1 << VA_WIDTH_SV39) - 1))
        } else {
            v.0
        }
    }
}
impl From<VirtPageNum> for usize {
    fn from(v: VirtPageNum) -> Self {
        v.0
    }
}
///kernel address impl
impl KernelAddr {
    ///Get mutable reference to `PhysAddr` value
    pub fn reinterpret<T>(&self) -> &'static T {
        // 将`self.0`转换为`*mut T`类型，然后使用`as_ref()`方法获取`&T`类型的可变引用
        unsafe { (self.0 as *mut T).as_ref().unwrap() }
    }
    ///Get mutable reference to `PhysAddr` value
    pub fn reinterpret_mut<T>(&self) -> &'static mut T {
        // 将`self.0`转换为`*mut T`类型，然后使用`as_mut()`方法获取`&mut T`类型的可变引用
        unsafe { (self.0 as *mut T).as_mut().unwrap() }
    }
}
/// virtual address impl
impl VirtAddr {
    /// Get the (floor) virtual page number
    pub fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / PAGE_SIZE)
    }

    /// Get the (ceil) virtual page number
    pub fn ceil(&self) -> VirtPageNum {
        assert!(self.0!=0);
        VirtPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }

    /// Get the page offset of virtual address
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    /// Check if the virtual address is aligned by page size
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}
impl From<VirtAddr> for VirtPageNum {
    fn from(v: VirtAddr) -> Self {
        if v.page_offset()!=0{
           panic!("virtual address is not aligned by page size!");
        }
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<VirtPageNum> for VirtAddr {
    fn from(v: VirtPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}
impl PhysAddr {
    #[inline]
    pub fn slice_with_len<T>(&self, len: usize) -> &'static [T] {
        unsafe { core::slice::from_raw_parts(self.get_ptr(), len) }
    }

    #[inline]
    pub fn slice_mut_with_len<T: Sized>(&self, len: usize) -> &'static mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.get_mut_ptr(), len) }
    }
    /// Get the (floor) physical page number
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / PAGE_SIZE)
    }
    /// Get the (ceil) physical page number
    pub fn ceil(&self) -> PhysPageNum {
        PhysPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }
    /// Get the page offset of physical address
    pub fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }
    /// Check if the physical address is aligned by page size
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}
impl From<PhysAddr> for PhysPageNum {
    fn from(v: PhysAddr) -> Self {
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<PhysPageNum> for PhysAddr {
    fn from(v: PhysPageNum) -> Self {
        Self(v.0 << PAGE_SIZE_BITS)
    }
}

impl VirtPageNum {
    #[cfg(target_arch = "riscv64")]
    /// Get the indexes of the page table entry
    pub fn indexes(&self) -> [usize; 3] {
        let mut vpn = self.0;
        let mut idx = [0usize; 3];
        for i in (0..3).rev() {
            idx[i] = vpn & 511;
            vpn >>= 9;
        }
        idx
    }
    #[cfg(target_arch = "loongarch64")]
    #[cfg(target_arch = "loongarch64")]
    pub fn indexes(&self) -> [usize; 4] {
        let mut vpn = self.0;
        let mut idx = [0usize; 4];
        
        idx[0] = (vpn >> 27) & 0x1ff; // PGD [47:39]
        idx[1] = (vpn >> 18) & 0x1ff; // PUD [38:30]
        idx[2] = (vpn >> 9) & 0x1ff;  // PMD [29:21]
        idx[3] = vpn & 0x1ff;         // PTE [20:12]
        
        idx
    }
}

impl PhysAddr {
    ///Get reference to `PhysAddr` value
    pub fn get_ref<T>(&self) ->  &'static T {
        // unsafe { (self.0 as *const T).as_ref().unwrap() }
        KernelAddr::from(*self).get_ref()
    }
    ///Get mutable reference to `PhysAddr` value
    pub fn get_mut<T>(&self) -> &'static mut T {
        // unsafe { (self.0 as *mut T).as_mut().unwrap() }
        KernelAddr::from(*self).get_mut()
    }
    pub fn get_ptr<T>(&self)->*const T{
        KernelAddr::from(*self).get_ptr()
    }
    pub fn get_mut_ptr<T>(&self) -> *mut T {
        
        KernelAddr::from(*self).get_mut_ptr()
    }
}

/// impl KernelAddr
impl KernelAddr {
    /// 定义一个公共函数 `as_ref`，它接受一个泛型参数 `T`
    pub fn get_ref<T>(&self) -> &'static T {
        unsafe { (self.0 as *const T).as_ref().unwrap() }
    }
    
    pub fn get_ptr<T>(&self)-> *const T{
        self.0 as *const T
    }
    /// 定义一个公共函数 `as_mut`，它接受一个泛型参数 `T`，并返回一个可变引用 `&'static mut T`
    ///Get mutable reference to `PhysAddr` value
    pub fn get_mut<T>(&self) -> &'static mut T {
        unsafe { (self.0 as *mut T).as_mut().unwrap() }
    }
    pub fn get_mut_ptr<T>(&self)->*mut T {
        self.0 as *mut T
    }
}

impl PhysPageNum {
    /// Get the reference of page table(array of ptes)
    pub fn get_pte_array(&self) -> &'static mut [PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        let kernel_va = KernelAddr::from(pa).0;

        unsafe { core::slice::from_raw_parts_mut(kernel_va as *mut PageTableEntry, 512) }
    
    }
    /// Get the reference of page(array of bytes)
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        let kernel_va = KernelAddr::from(pa).0;
        
        unsafe { core::slice::from_raw_parts_mut(kernel_va as *mut u8, 4096) }
     
      
    }
    /// Get the mutable reference of physical address
    pub fn get_mut<T>(&self) -> &'static mut T {
        let pa: PhysAddr = (*self).into();
        let kernel_va = KernelAddr::from(pa);
        kernel_va.get_mut()
    }
  /// Get the mutable reference of physical address
    pub fn get_mut_ptr<T>(&self) -> * mut T {
        let pa: PhysAddr = (*self).into();
        let kernel_va = KernelAddr::from(pa);
        kernel_va.get_mut_ptr()
    }
}

/// iterator for phy/virt page number
pub trait StepByOne {
    /// step by one element(page number)
    fn step(&mut self);
}
impl StepByOne for VirtPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}
impl StepByOne for PhysPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}

#[derive(Copy, Clone)]
/// a simple range structure for type T
pub struct SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    l: T,
    r: T,
}
impl<T> SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    pub fn new(start: T, end: T) -> Self {
       assert! (start <= end, "start {:?} > end {:?}!", start, end);
        Self { l: start, r: end }
    }
    pub fn get_start(&self) -> T {
        self.l
    }
    pub fn get_end(&self) -> T {
        self.r
    }
    pub fn range(&self) -> (T, T) {
        (self.l, self.r)
    }
    pub fn contains(&self,val : T) -> bool{
        self.l<=val&&self.r>=val
    }
    pub fn empty(&self)->bool{
        self.l==self.r
    }
    pub fn set_end(&mut self,val:T){
        assert!(self.get_start()<=val);
        self.r=val;
    }
    pub fn iter(&self) -> SimpleRangeIterator<T> {
        SimpleRangeIterator::new(self.l, self.r)
    }
 
}
impl<T> IntoIterator for SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    type Item = T;
    type IntoIter = SimpleRangeIterator<T>;
    fn into_iter(self) -> Self::IntoIter {
        SimpleRangeIterator::new(self.l, self.r)
    }
}
/// iterator for the simple range structure
pub struct SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    current: T,
    end: T,
}
impl<T> SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    pub fn new(l: T, r: T) -> Self {
        Self { current: l, end: r }
    }
}
impl<T> Iterator for SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + Debug,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.end {
            None
        } else {
            let t = self.current;
            self.current.step();
            Some(t)
        }
    }
}

impl Add for PhysAddr {
    type Output = PhysAddr;
    fn add(self, rhs: Self) -> Self::Output {
        PhysAddr(self.0 + rhs.0)
    }
}
impl Add<usize> for PhysAddr {
    type Output = PhysAddr;
    fn add(self, rhs: usize) -> Self::Output {
        PhysAddr(self.0 + rhs)
    }
}
impl Sub for PhysAddr {
    type Output = PhysAddr;
    fn sub(self, rhs: Self) -> Self::Output {
        PhysAddr(self.0 - rhs.0)
    }
}
impl Sub<usize> for PhysAddr {
    type Output = PhysAddr;
    fn sub(self, rhs: usize) -> Self::Output {
        PhysAddr(self.0 - rhs)
    }
}
impl Add for VirtAddr {
    type Output = VirtAddr;
    fn add(self, rhs: Self) -> Self::Output {
        VirtAddr(self.0 + rhs.0)
    }
}
impl Add<usize> for VirtAddr {
    type Output = VirtAddr;
    fn add(self, rhs: usize) -> Self::Output {
        VirtAddr(self.0 + rhs)
    }
}
impl Sub for VirtAddr {
    type Output = VirtAddr;
    fn sub(self, rhs: Self) -> Self::Output {
        VirtAddr(self.0 - rhs.0)
    }
}
impl Sub<usize> for VirtAddr {
    type Output = VirtAddr;
    fn sub(self, rhs: usize) -> Self::Output {
        VirtAddr(self.0 - rhs)
    }
}


/// a simple range structure for virtual page number
pub type VPNRange = SimpleRange<VirtPageNum>;
