#[repr(C)]
#[derive(Debug, Default,Clone, Copy)]
pub struct Kstat {
    pub st_dev: usize,  // 包含文件的设备 ID
    pub st_ino: usize,  // 索引节点号
    pub st_mode: u32,   // 文件类型和模式
    pub st_nlink: u32,  // 硬链接数
    pub st_uid: u32,    // 所有者的用户 ID
    pub st_gid: u32,    // 所有者的组 ID
    pub st_rdev: usize, // 设备 ID（如果是特殊文件）
    pub __pad: usize,
    pub st_size: isize,  // 总大小，以字节为单位
    pub st_blksize: i32, // 文件系统 I/O 的块大小
    pub __pad2: u32,
    pub st_blocks: isize,     // 分配的 512B 块数
    pub st_atime: isize,      // 上次访问时间
    pub st_atime_nsec: usize, // 上次访问时间（纳秒精度）
    pub st_mtime: isize,      // 上次修改时间
    pub st_mtime_nsec: usize, // 上次修改时间（纳秒精度）
    pub st_ctime: isize,      // 上次状态变化的时间
    pub st_ctime_nsec: usize, // 上次状态变化的时间（纳秒精度）
    pub __unused: [u32; 2],
}


#[repr(C)]
#[derive(Debug, Default,Clone, Copy)]
pub struct Statfs {
    pub f_type: i64,       // Type of filesystem
    pub f_bsize: i64,      // Optimal transfer block size
    pub f_blocks: i64,     // Total data blocks in filesystem
    pub f_bfree: i64,      // Free blocks in filesystem
    pub f_bavail: i64,     // Free blocks available to unprivileged user
    pub f_files: i64,      // Total inodes in filesystem
    pub f_ffree: i64,      // Free inodes in filesystem
    pub f_fsid: i64,       // Filesystem ID
    pub f_name_len: i64,   // Maximum length of filenames
    pub f_frsize: i64,     // Fragment size
    pub f_flags: i64,      // Mount flags of filesystem
    pub f_spare: [i64; 4], // Padding bytes
}
bitflags! {
    pub struct StMode: u32 {
        const FIFO= 0x1000; //管道设备文件
        const FCHR = 0x2000; //字符设备文件
        const FDIR = 0x4000; //目录文件
        const FBLK = 0x6000; //块设备文件
        const FREG = 0x8000; //普通文件
        const FLINK = 0xA000; //符号链接文件
        const FSOCK = 0xC000; //套接字设备文件
    }
}
impl From<&Kstat> for StatxTimestamp {
    // 这个实现是临时的，只为了 atime。理想情况下，
    // Kstat 也应该有 atime, mtime, ctime 的独立字段。
    // 为了编译通过，我们先这样写。
    fn from(kstat: &Kstat) -> Self {
        StatxTimestamp {
            tv_sec: kstat.st_atime as i64,
            tv_nsec: kstat.st_atime_nsec as u32,
            __reserved: 0,
        }
    }
}


// --- 核心的转换实现 ---

impl From<&Kstat> for Statx {
    fn from(kstat: &Kstat) -> Self {
        let mut statx = Statx::default();

        // 1. 文件类型和模式 (stx_mode)
        // kstat.st_mode 同时包含了文件类型（高位）和权限（低位）
        // StMode bitflags 只定义了文件类型，我们需要它们
        let file_type_mode = StMode::from_bits_truncate(kstat.st_mode).bits();
        let perms_mode = kstat.st_mode & 0o7777; // 取出 rwx 权限位
        statx.stx_mode = (file_type_mode | perms_mode) as u16;

        // 2. 链接数、UID、GID
        statx.stx_nlink = kstat.st_nlink;
        statx.stx_uid = kstat.st_uid;
        statx.stx_gid = kstat.st_gid;

        // 3. Inode 号
        statx.stx_ino = kstat.st_ino as u64;

        // 4. 文件大小
        statx.stx_size = kstat.st_size as u64; // kstat.st_size 是 isize

        // 5. 块大小和块数量
        statx.stx_blksize = kstat.st_blksize as u32;
        // statx 要求块大小是 512B，而 kstat.st_blocks 的单位
        // 是 kstat.st_blksize。我们需要进行转换。
        // (kstat.st_blocks * kstat.st_blksize) / 512
        if kstat.st_blksize > 0 {
             statx.stx_blocks = (kstat.st_blocks as u64)
                               .wrapping_mul(kstat.st_blksize as u64) 
                               / 512;
        } else {
            // 如果 blksize 无效，给一个合理的回退
            statx.stx_blocks = (kstat.st_size as u64 + 511) / 512;
        }
       
        // 6. 时间戳转换
        // StatxTimestamp 有 tv_sec (i64) 和 tv_nsec (u32)
        // Kstat 使用 isize 和 usize
        statx.stx_atime = StatxTimestamp {
            tv_sec: kstat.st_atime as i64,
            tv_nsec: kstat.st_atime_nsec as u32,
            ..Default::default()
        };
        statx.stx_mtime = StatxTimestamp {
            tv_sec: kstat.st_mtime as i64,
            tv_nsec: kstat.st_mtime_nsec as u32,
            ..Default::default()
        };
        statx.stx_ctime = StatxTimestamp {
            tv_sec: kstat.st_ctime as i64,
            tv_nsec: kstat.st_ctime_nsec as u32,
            ..Default::default()
        };
        
        // btime (创建时间) 通常不被所有文件系统支持，可以留空
        // statx.stx_btime = ...;

  
        statx.stx_dev_major = (kstat.st_dev >> 8) as u32;
        statx.stx_dev_minor = (kstat.st_dev & 0xff) as u32;
        statx.stx_rdev_major = (kstat.st_rdev >> 8) as u32;
        statx.stx_rdev_minor = (kstat.st_rdev & 0xff) as u32;

        // 8. 属性字段 (attributes) - 通常可以先留空
        // statx.stx_attributes = ...;
        // statx.stx_attributes_mask = ...;

        statx
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
    pub __reserved: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Statx {
    pub stx_mask: u32,            // [in, out] Mask of fields requested/filled
    pub stx_blksize: u32,         // Block size for filesystem I/O
    pub stx_attributes: u64,      // Extra file attribute indicators
    pub stx_nlink: u32,           // Number of hard links
    pub stx_uid: u32,             // User ID of owner
    pub stx_gid: u32,             // Group ID of owner
    pub stx_mode: u16,            // File type and mode
    pub __spare0: [u16; 1],
    pub stx_ino: u64,             // Inode number
    pub stx_size: u64,            // Total size in bytes
    pub stx_blocks: u64,          // Number of 512B blocks allocated
    pub stx_attributes_mask: u64, // Mask to show what's supported in stx_attributes

    // Timestamps
    pub stx_atime: StatxTimestamp, // Last access
    pub stx_ctime: StatxTimestamp, // Last status change
    pub stx_mtime: StatxTimestamp, // Last modification
    
    // btime is "birth time" or creation time
    pub stx_btime: StatxTimestamp, // Creation

    // Device ID of filesystem
    pub stx_dev_major: u32,
    pub stx_dev_minor: u32,
    
    // Device ID of special file
    pub stx_rdev_major: u32,
    pub stx_rdev_minor: u32,

    pub __spare2: [u64; 14],
}
