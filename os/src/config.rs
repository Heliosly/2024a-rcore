//! Constants in the kernel

#[allow(unused)]

///最大解析深度
pub const MAX_SYMLINK_DEPTH: usize = 100;
pub const MAX_SHM_SIZE: usize = 0x4000_0000;
///
pub const MNT_TABLE_MAX_ENTRIES: usize = 16;
///File descriptor set size
pub const FD_SETSIZE: usize = 1024;
//  MAX_KERNEL_RW_BUFFER_SIZE
pub const MAX_KERNEL_RW_BUFFER_SIZE: usize = 4096 * 4;
/// Signal information. Corresponds to `struct siginfo_t` in libc.
pub const SS_DISABLE: u32 = 2;
/// maximum number of readv/writev iovecs
pub const UIO_MAXIOV: usize = 1024;

pub const USER_SIGNAL_PROTECT: usize = 512;
/// kernel stack size
pub const KERNEL_STACK_SIZE: usize = 4096 * 16 * 10;
/// kernel heap size
pub const KERNEL_HEAP_SIZE: usize = 134217728;
///于allocuserres中于分配页数
pub const PRE_ALLOC_PAGES: usize = 8;

/// the max number of syscall
pub const MAX_SYSCALL_NUM: usize = 500;
/// the virtual addr of trapoline
pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
/// clock frequency
pub const CLOCK_FREQ: usize = 12500000;
/// the physical memory end
pub const MEMORY_END: usize = KERNEL_DIRECT_OFFSET + 0xB000_0000;
// For RISC-V QEMU virt machine (MMIO VirtIO)
#[cfg(target_arch = "riscv64")]
pub const MMIO: &[(usize, usize)] = &[
    (0x02000000, 0x00200000), // CLINT
    (0x0C000000, 0x04000000), // PLIC
    (0x10001000, 0x00001000), // VirtIO Block
    (0x10002000, 0x00001000), // VirtIO Net
                              // ... other MMIO devices for RISC-V
];

// For LoongArch64 QEMU virt machine (PCI-E VirtIO)
#[cfg(target_arch = "loongarch64")]
pub const MMIO: &[(usize, usize)] = &[
    // 核心平台设备和中断控制器区域
    // Physical address 0x10000000 contains the Loongson PIC and other platform devices.
    // We map a large chunk to be safe. Let's map 256MB.
    (0x1000_0000, 0x1000_0000),
    // PCI-E Enhanced Configuration Access Mechanism (ECAM) space.
    // Physical address 0x20000000 is the base of the ECAM region.
    // The standard size is 256MB.
    (0x2000_0000, 0x1000_0000),
    // VirtIO devices over PCI get their BARs mapped dynamically.
    // However, some platforms might have fixed regions for BARs.
    // Let's pre-map the region where we plan to allocate BARs.
    // In your `dump_bar_contents` function, you used 0x4000_0000 as a base.
    // Let's map that region as well. Let's map 256MB for BARs.
    (0x4000_0000, 0x1000_0000),
];
/// Kerneladress offset
pub const KERNEL_DIRECT_OFFSET: usize = 0x9000_0000_0000_0000;
/// When directly map: vpn = ppn + kernel direct offset
pub const KERNEL_PGNUM_OFFSET: usize = KERNEL_DIRECT_OFFSET >> PAGE_SIZE_BITS;

/// 定义协程堆栈的大小，  40MB
pub const TASK_STACK_SIZE: usize = 0x400000;
/// 定义用户空间的总大小，48GB
pub const USER_SPACE_SIZE: usize = 0x30_0000_0000;
/// 定义最大线程数，3000
pub const THREAD_MAX_NUM: usize = 3000;
/// User Space layout
/// TrapContext GuardPage Stack GuardPage Mmap Heap Elf
/// 定义用户空间布局的顶部地址，即用户空间的起始地址
pub const USER_TRAP_CONTEXT_TOP: usize = USER_SPACE_SIZE;

/// 定义用户栈的顶部地址
/// 用户栈的顶部地址是用户空间布局的顶部地址减去线程最大数乘以页面大小
pub const USER_STACK_TOP: usize = USER_TRAP_CONTEXT_TOP - PAGE_SIZE * THREAD_MAX_NUM;

/// 定义内存映射（Mmap）区域的顶部地址
/// 内存映射区域的顶部地址也是用户空间布局的顶部地址减去线程最大数乘以页面大小
pub const MMAP_TOP: usize = USER_TRAP_CONTEXT_TOP
    - PAGE_SIZE * THREAD_MAX_NUM
    - USER_STACK_SIZE * THREAD_MAX_NUM
    - PAGE_SIZE;
pub const MMAP_PGNUM_TOP: usize = MMAP_TOP >> PAGE_SIZE_BITS;
/// user app's heap size 4mb
pub const USER_HEAP_SIZE: usize = 0x40000;

pub const MMAP_BASE: usize = MMAP_TOP - (134217728) * 4;
/// user app's stack size
pub const USER_STACK_SIZE: usize = 4096 * 16 * 16 * 10;
/// Kernel Stack Start
pub const KSTACK_TOP: usize = usize::MAX - PAGE_SIZE + 1;
///temp data
pub const IS_ASYNC: usize = 0x5f5f5f5f;

/// Dynamic linked interpreter address range in user space
pub const DL_INTERP_OFFSET: usize = 0x15_0000_0000;

///Max Fd
pub const MAX_FD_NUM: usize = 100;

// Maximum path length
pub const PATH_MAX: usize = 4096;

// 定义一个内核中转缓冲区的合理大小
pub const SENDFILE_KERNEL_BUFFER_SIZE: usize = 4 * PAGE_SIZE;
pub const TOTALMEM: usize = 1 * 1024 * 1024 * 1024; // 1 GiB
pub const PAGE_SIZE: usize = 0x1000;
pub const PAGE_SIZE_BITS: usize = 12;
cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")] {
         /// page size : 4KB

    } else if #[cfg(target_arch = "loongarch64")] {

    } else {
        compile_error!("Unsupported target architecture");
    }
}
