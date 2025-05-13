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
#[derive(Debug, Default)]
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
