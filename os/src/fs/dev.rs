/// 该模块实现了`/dev`目录下的各种设备文件，
/// 提供了对特殊内核功能和硬件接口的访问。
///
/// 它包括以下实现的设备：
/// - `/dev/zero`: 提供无限空字节流的设备。
/// - `/dev/null`: 丢弃所有写入它的数据，读取时没有数据提供的设备。
/// - `/dev/rtc`: 用于读取当前日期和时间的实时时钟设备。
/// - `/dev/random`: 提供随机数流的设备。
/// - `/dev/tty`: 用于与标准输入和标准输出交互的终端设备。
/// - `/dev/cpu_dma_latency`: 用于获取/设置CPU最大反应时间的设备。
///
/// 该模块使用设备树（`DEVICES`）将设备名称映射到设备编号，
/// 允许动态注册和查找设备。每个设备都实现了`File` trait，
/// 使其可以在虚拟文件系统中使用。
///
/// # 设备注册
///
/// 设备使用`register_device`函数注册，该函数将条目添加到
/// `DEVICES`树，将设备名称与唯一的设备编号相关联。
/// `unregister_device`函数从树中删除设备。
///
/// # 设备访问
///
/// `open_device_file`函数用于获取给定设备路径的`Arc<dyn File>`。
/// 这允许像打开常规文件一样打开设备并与之交互。
///
/// # 设备实现
///
/// 每个设备实现都为其读取、写入和其他文件操作提供其自己的特定行为。
/// 例如，`DevZero`在读取时用空字节填充用户缓冲区，而`DevNull`丢弃写入它的任何数据。
/// `DevRandom`用随机字节填充用户缓冲区。`DevRtc`提供当前时间。
/// `DevTty`提供终端界面。`DevCpuDmaLatency`允许用户获取/设置CPU的最大反应时间。
use crate::{
    mm::{MapPermission, MmapFlags, UserBuffer, VirtAddr}, syscall::flags::MmapProt, task::current_process, utils::error::{SysErrNo, SyscallRet, TemplateRet}
   
};
use alloc::{
    collections::BTreeMap,
    fmt::{Debug, Formatter},
    format,
    string::{String, ToString},
    sync::Arc,
};
use async_trait::async_trait;
use linux_raw_sys::general::xattr_args;
use core::{cmp::min, task::Waker};
use spin::{Lazy, Mutex, RwLock};

use alloc::boxed::Box;
use super::{stat::StMode, File, Kstat, PollEvents, Stdin, Stdout};

pub struct DevZero;
pub struct DevNull;
pub struct DevRtc;
pub struct DevRandom;

pub struct DevTty;

pub struct DevCpuDmaLatency {
    reaction_time: RwLock<u32>, //进程最大反应时间,即CPU最大延迟,单位us
}

//设备树，通过设备名称可以查找到设备号
pub static DEVICES: Lazy<Mutex<BTreeMap<String, usize>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

//从1起算，0为其他抽象文件
static mut DEV_NO: usize = 1;

pub fn register_device(abs_path: &str) {
    unsafe {
        DEVICES.lock().insert(abs_path.to_string(), DEV_NO);
        info!("register device {},{}", abs_path, DEV_NO);
        DEV_NO += 1;
    }
}

pub fn unregister_device(abs_path: &str) {
    DEVICES.lock().remove(&abs_path.to_string());
}

pub fn find_device(abs_path: &str) -> bool {
   let a= DEVICES.lock().get(abs_path).is_some();
    // info!("find device {},{}", abs_path,a);
    a
}

pub fn get_devno(abs_path: &str) -> usize {
    *DEVICES.lock().get(abs_path).unwrap()
}

pub fn open_device_file(abs_path: &str) -> Result<Arc<dyn File>, SysErrNo> {
    match abs_path {
        "/dev/zero" => Ok(Arc::new(DevZero::new())),
        "/dev/null" => Ok(Arc::new(DevNull::new())),
        "/dev/rtc" | "/dev/rtc0" | "/dev/misc/rtc" => Ok(Arc::new(DevRtc::new())),
        "/dev/random" => Ok(Arc::new(DevRandom::new())),
        "/dev/tty" => Ok(Arc::new(DevTty::new())),
        "/dev/cpu_dma_latency" => Ok(Arc::new(DevCpuDmaLatency::new())),

       
        _ => Err(SysErrNo::ENOENT),
    }
}

/// zero设备
impl DevZero {
    pub fn new() -> Self {
        Self
    }

    fn get_path(&self) -> String {
        "/dev/zero".to_string()
    }
}

#[async_trait]
impl File for DevZero {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(
        &self,
        mut user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Fill buffer with zeros
        Ok(user_buf.fill0())
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // /dev/zero discards written data
        Ok(user_buf.len())
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/zero");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) {
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
        Err(SysErrNo::ESPIPE)
    }
}

/// NULL设备
impl DevNull {
    pub fn new() -> Self {
        Self
    }

    fn get_path(&self) -> String {
        "/dev/null".to_string()
    }
}
#[async_trait]
impl File for DevNull {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(
        &self,
        mut _user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // /dev/null returns EOF immediately
        Ok(0)
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // /dev/null discards written data
        Ok(user_buf.len())
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/null");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) {
            // always ready to read (EOF)
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence:u32) -> SyscallRet {
        Err(SysErrNo::ESPIPE)
    }
}


pub struct RtcTime {
    pub year: u32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl RtcTime {
    pub fn new(year: u32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }
}

impl Debug for RtcTime {
    
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}-{}-{} {}:{}:{}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// 时钟设备
impl DevRtc {
 
    pub fn new() -> Self {
        Self
    }

    fn get_path(&self) -> String {
        "/dev/rtc".to_string()
    }
}


#[async_trait]
impl File for DevRtc {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(
        &self,
        mut user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Return formatted RTC time string
        let time = RtcTime::new(2000, 1, 1, 0, 0, 0);
        let s = format!("{:?}", time);
        let bytes = s.as_bytes();
        let len = core::cmp::min(user_buf.len(), bytes.len());
        user_buf.write(&bytes[..len]);
        Ok(len)
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // /dev/rtc discards written data
        Ok(user_buf.len())
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/rtc");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) {
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
        Err(SysErrNo::ESPIPE)
    }
}

/// 随机数设备
impl DevRandom {
    pub fn new() -> Self {
        Self
    }

    fn get_path(&self) -> String {
        "/dev/random".to_string()
    }
}

#[async_trait]
impl File for DevRandom {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(
        &self,
        mut user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Fill buffer with random data
        Ok(user_buf.fillrandom())
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // /dev/random discards written data
        Ok(user_buf.len())
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/random");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        // Ready to read any time
        if events.contains(PollEvents::POLLIN) {
            revents |= PollEvents::POLLIN;
        }
        // Writing to random always possible
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        // Always report error availability
        if events.contains(PollEvents::POLLERR) {
            revents |= PollEvents::POLLERR;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence:u32) -> SyscallRet {
        Err(SysErrNo::ESPIPE)
    }
}

/// 终端设备
impl DevTty {
    pub fn new() -> Self {
        Self
    }

    fn get_path(&self) -> String {
        "/dev/tty".to_string()
    }
}



#[async_trait]
impl File for DevTty {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Forward read to console stdin
        Stdin.read(user_buf).await
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Forward write to console stdout
        Stdout.write(user_buf).await
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/tty");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        if events.contains(PollEvents::POLLIN) {
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        if events.contains(PollEvents::POLLERR) {
            revents |= PollEvents::POLLERR;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
        Err(SysErrNo::ESPIPE)
    }
}

/// cpu频率设备
impl DevCpuDmaLatency {
    pub fn new() -> Self {
        Self {
            reaction_time: RwLock::new(10),
        }
    }

    fn get_path(&self) -> String {
        "/dev/cpu_dma_latency".to_string()
    }
}

#[async_trait]
impl File for DevCpuDmaLatency {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        // Always readable
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        // Always writable
        Ok(true)
    }

    async fn read<'a>(
        &self,
        mut user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // Read 4-byte reaction_time in big-endian order
        let reaction_time = *self.reaction_time.read();
        let buf = [
            (reaction_time >> 24) as u8,
            (reaction_time >> 16) as u8,
            (reaction_time >> 8) as u8,
            reaction_time as u8,
        ];
        // Write at most buffer length
        let len = min(user_buf.len(), buf.len());
        Ok(user_buf.write(&buf[..len]))
    }

    async fn write<'a>(
        &self,
        user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        let mut bytes: [u8; 4] = [0; 4];
        let mut count = 0;
        for sub_buff in user_buf.buffers.iter() {
            let sblen = (*sub_buff).len();
            for j in 0..sblen {
                bytes[count] = (*sub_buff)[j];
                count += 1;
            }
        }
        let mut reaction_time = self.reaction_time.write();
        *reaction_time = (bytes[0] as u32) << 24
            | (bytes[1] as u32) << 16
            | (bytes[2] as u32) << 8
            | bytes[3] as u32;
        Ok(4)
    }

    fn fstat(&self) -> Kstat {
        let devno = get_devno("/dev/cpu_dma_latency");
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits(),
            st_rdev: devno,
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, _waker: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        // indicate readiness based on bitflags
        if events.contains(PollEvents::POLLIN) {
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) {
            revents |= PollEvents::POLLOUT;
        }
        if events.contains(PollEvents::POLLERR) {
            revents |= PollEvents::POLLERR;
        }
        revents
    }

    fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
        // Seeking is not supported on this device
        Err(SysErrNo::ESPIPE)
    }
    
}



pub struct DevShm;

impl DevShm {
    pub fn new() -> Self {
        Self
    }
    /// 关键实现：处理 mmap 系统调用
    /// 当用户对 /dev/shm 的文件描述符调用 mmap 时，此函数被触发。
    /// 我们利用 offset 参数作为共享内存的 key。
    pub async fn mmap(
        &self,
        addr: usize,    // 建议的映射地址
        len: usize,     // 映射长度
        prot: MapPermission,    // 保护位 (PROT_READ, PROT_WRITE, etc.)
        flags: MmapFlags,   // 标志位 (MAP_SHARED, MAP_PRIVATE, etc.)
        offset: usize,  // **我们将用 offset 作为共享内存的 key **
    ) -> Result<usize, SysErrNo> {
        info!("[Devshm mmap]");
        // 1. 参数校验
        if len == 0 || len > crate::config::MAX_SHM_SIZE {
            return Err(SysErrNo::EINVAL);
        }
        // mmap 到 /dev/shm 必须是 MAP_SHARED
        if flags .contains(MmapFlags::MAP_SHARED) {
            return Err(SysErrNo::EINVAL);
        }
        // offset 作为 key，我们将其转换为 i32
        let key = offset as i32;

        // 2. 获取或创建共享内存段
        let mut manager = crate::mm::shm::SHM_MANAGER.lock().await;

        let shmid = match manager.key_to_id.get(&key) {
            // --- 情况 A: 共享内存已存在 ---
            Some(&existing_shmid) => {
                let segment_arc = manager.id_to_segment.get(&existing_shmid).unwrap();
                let segment = segment_arc.lock();
                // 检查请求的长度是否超过段大小
                if len > segment.id_ds.shm_segsz {
                    return Err(SysErrNo::EINVAL);
                }
                existing_shmid
            }
            // --- 情况 B: 创建新的共享内存 ---
            None => {
                let pid = current_process().pid.0;
                let new_segment = match crate::mm::shm::SharedMemorySegment::new(key, len, pid) {
                    Some(seg) => seg,
                    None => return Err(SysErrNo::ENOMEM), // 物理内存分配失败
                };
                let new_shmid = manager.next_id.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as i32;
                let segment_arc = Arc::new(crate::sync::Mutex::new(new_segment));

                manager.id_to_segment.insert(new_shmid, segment_arc);
                // 对于 /dev/shm 模型，key 总是公开的
                manager.key_to_id.insert(key, new_shmid);
                new_shmid
            }
        };
        

        let segment_arc = manager.id_to_segment.get(&shmid).unwrap().clone();
        drop(manager); // 释放管理器锁

        let process = current_process();
        let mut ms = process.memory_set.lock();
        let segment = segment_arc.lock();

        let page_count = (len + crate::config::PAGE_SIZE - 1) / crate::config::PAGE_SIZE;

        // 分配虚拟地址空间
        let start_vpn = ms.areatree.alloc_pages_from_hint(page_count, VirtAddr::from(addr).ceil())
            .ok_or(SysErrNo::ENOMEM)?;
        let end_vpn = crate::mm::VirtPageNum(start_vpn.0 + page_count);

        // 创建 MapArea
        
        let map_area = crate::mm::MapArea::new_by_vpn(
            start_vpn,
            end_vpn,
            crate::mm::MapType::Framed, 
            prot|MapPermission::U,
            crate::mm::MapAreaType::Shm { shmid }, // 存储 shmid 以便 munmap
        );

        // 准备物理帧映射
        let map: BTreeMap<crate::mm::VirtPageNum, Arc<crate::mm::FrameTracker>> = segment
            .frames
            .iter()
            .take(page_count) // 只映射请求的长度对应的页
            .enumerate()
            .map(|(i, frame)| (crate::mm::VirtPageNum(start_vpn.0 + i), frame.clone()))
            .collect();
        
        ms.push_with_given_frames(map_area, &map, false);

        // 更新段的附加信息
        drop(segment);
        segment_arc.lock().attach(process.pid.0 as u32);
        
        Ok(VirtAddr::from(start_vpn).0)
    }
}

#[async_trait]
impl File for DevShm {
    
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        // /dev/shm 本身不可读
        Ok(false)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        // /dev/shm 本身不可写
        Ok(false)
    }

    async fn read<'a>(
        &self,
        _user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // 对设备本身的读操作没有意义
        Err(SysErrNo::EINVAL)
    }

    async fn write<'a>(
        &self,
        _user_buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        // 对设备本身的写操作没有意义
        Err(SysErrNo::EINVAL)
    }

    fn lseek(&self, _offset: isize, _whence: u32) -> SyscallRet {
        // 不支持 seek
        Err(SysErrNo::ESPIPE)
    }

    fn fstat(&self) -> Kstat {
        // 提供一个典型的字符设备元数据
        let devno = get_devno("/dev/shm"); // 假设您有这样一个函数获取设备号
        Kstat {
            st_dev: devno,
            st_mode: StMode::FCHR.bits() | 0o666, // 字符设备，权限 rw-rw-rw-
            st_rdev: devno,
            st_nlink: 1,
            st_size: 0, // 设备文件大小为 0
            ..Kstat::default()
        }
    }
    
    // poll 方法可以像您的例子一样实现，表示总是可读写（虽然实际操作会失败）
    // 或者更准确地，返回错误。
    fn poll(&self, _events: PollEvents, _waker: &Waker) -> PollEvents {
        // 报告错误，因为常规的 I/O 不被支持
        PollEvents::POLLERR
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self 
    }
    fn get_path(&self)->String{
        return "/dev/shm/cyclictest9".to_string()
    }
}