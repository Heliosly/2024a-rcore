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
    mm::UserBuffer, utils::error::{SysErrNo, SyscallRet, TemplateRet},
   
};
use alloc::{
    collections::BTreeMap,
    fmt::{Debug, Formatter},
    format,
    string::{String, ToString},
    sync::Arc,
};
use async_trait::async_trait;
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
}

#[async_trait]
impl File for DevZero {
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
}
#[async_trait]
impl File for DevNull {
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
}


#[async_trait]
impl File for DevRtc {
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
}

#[async_trait]
impl File for DevRandom {
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
}



#[async_trait]
impl File for DevTty {
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
}

#[async_trait]
impl File for DevCpuDmaLatency {
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
