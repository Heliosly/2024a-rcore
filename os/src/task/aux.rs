#[derive(Clone, Copy)]
#[allow(non_camel_case_types, unused)]

/// ELF 辅助向量类型（对应 Linux 下的 <elf.h> 中的注释）
#[repr(usize)]
#[derive(Debug,PartialEq, Eq)]
pub enum AuxType {
    /// 辅助向量结束标志
    NULL = 0,
    /// 忽略该条目
    IGNORE = 1,
    /// 可执行文件的文件描述符
    EXECFD = 2,
    /// 程序头表的起始地址
    PHDR = 3,
    /// 每个程序头项的大小
    PHENT = 4,
    /// 程序头项的数量
    PHNUM = 5,
    /// 系统页面大小
    PAGESZ = 6,
    /// 解释器（动态链接器）的基地址
    BASE = 7,
    /// 标志（未使用）
    FLAGS = 8,
    /// 可执行文件的入口地址
    ENTRY = 9,
    /// 不是 ELF 可执行文件的标识
    NOTELF = 10,
    /// 实际用户 ID
    UID = 11,
    /// 有效用户 ID
    EUID = 12,
    /// 实际组 ID
    GID = 13,
    /// 有效组 ID
    EGID = 14,
    /// 指示用于优化的 CPU 标识字符串
    PLATFORM = 15,
    /// CPU 硬件能力标志
    HWCAP = 16,
    /// times() 调用的时钟滴答频率（已废弃）
    CLKTCK = 17,
    /// FPU 控制字
    FPUCW = 18,
    /// 数据缓存块大小
    DCACHEBSIZE = 19,
    /// 指令缓存块大小
    ICACHEBSIZE = 20,
    /// 统一缓存块大小
    UCACHEBSIZE = 21,
    /// PPC 平台保留
    IGNOREPPC = 22,
    /// 安全模式标识
    SECURE = 23,
    /// 替代 PLATFORM 的真实平台标识
    BASE_PLATFORM = 24,
    /// 指向 16 字节随机值的地址
    RANDOM = 25,
    /// 扩展的 CPU 硬件能力标志
    HWCAP2 = 26,
    /// 可执行文件的文件名
    EXECFN = 31,
    /// 指向 VDSO sysinfo 页面
    SYSINFO = 32,
    /// VDSO ELF 头部地址
    SYSINFO_EHDR = 33,
    /// 一级指令缓存形状描述
    L1I_CACHESHAPE = 34,
    /// 一级数据缓存形状描述
    L1D_CACHESHAPE = 35,
    /// 二级缓存形状描述
    L2_CACHESHAPE = 36,
    /// 三级缓存形状描述
    L3_CACHESHAPE = 37,
    /// 一级指令缓存大小
    L1I_CACHESIZE = 40,
    /// 一级指令缓存几何信息
    L1I_CACHEGEOMETRY = 41,
    /// 一级数据缓存大小
    L1D_CACHESIZE = 42,
    /// 一级数据缓存几何信息
    L1D_CACHEGEOMETRY = 43,
    /// 二级缓存大小
    L2_CACHESIZE = 44,
    /// 二级缓存几何信息
    L2_CACHEGEOMETRY = 45,
    /// 三级缓存大小
    L3_CACHESIZE = 46,
    /// 三级缓存几何信息
    L3_CACHEGEOMETRY = 47,
    /// 最低可捕获信号栈大小
    MINSIGSTKSZ = 51,
}

#[derive(Debug)]
pub struct Aux {
    pub aux_type: AuxType,
    pub value: usize,
}
impl Aux {
    pub fn new(aux_type: AuxType, value: usize) -> Self {
        Self { aux_type, value }
    }
}
