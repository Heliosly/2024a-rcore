//! block device driver



mod virtio;
use crate::devices::get_blk_device;
use lwext4_rust::KernelDevOp;
// pub use virtio::loongson::IRQ_HANDLERS
use crate::{ utils::error::{SysErrNo, TemplateRet}};

const BLOCK_SIZE: usize = 0x200;
pub type Ext4Disk=Ext4DiskWrapper;
pub struct Ext4DiskWrapper {
    block_id: usize,
    offset: usize,
    blk_id: usize,
}

impl Ext4DiskWrapper {
    /// Create a new disk.
    pub const fn new(blk_id: usize) -> Self {
        Self {
            block_id: 0,
            offset: 0,
            blk_id,
        }
    }

    /// Get the position of the cursor.
    #[inline]
    pub fn position(&self) -> u64 {
        (self.block_id * BLOCK_SIZE + self.offset) as u64
    }

    /// Set the position of the cursor.
    #[inline]
    pub fn set_position(&mut self, pos: u64) {
        self.block_id = pos as usize / BLOCK_SIZE;
        self.offset = pos as usize % BLOCK_SIZE;
    }
}

impl KernelDevOp for Ext4DiskWrapper {
    type DevType = Self;

    fn write(dev: &mut Self::DevType, buf: &[u8]) -> Result<usize, i32> {
        assert!(dev.offset % BLOCK_SIZE == 0);
        get_blk_device(dev.blk_id)
            .expect("can't find block device")
            .write_blocks(dev.block_id, buf);
        dev.block_id += buf.len() / BLOCK_SIZE;
        Ok(buf.len())
    }

    fn read(dev: &mut Self::DevType, buf: &mut [u8]) -> Result<usize, i32> {
        assert!(dev.offset % BLOCK_SIZE == 0);
        get_blk_device(dev.blk_id)
            .expect("can't find block device")
            .read_blocks(dev.block_id, buf);
        dev.block_id += buf.len() / BLOCK_SIZE;
        Ok(buf.len())
    }

    fn seek(dev: &mut Self::DevType, off: i64, whence: i32) -> Result<i64, i32> {
        let size = get_blk_device(dev.blk_id)
            .expect("can't seek to device")
            .capacity();
        let new_pos = match whence as u32 {
            lwext4_rust::bindings::SEEK_SET => Some(off),
            lwext4_rust::bindings::SEEK_CUR => {
                dev.position().checked_add_signed(off).map(|v| v as i64)
            }
            lwext4_rust::bindings::SEEK_END => size.checked_add_signed(off as _).map(|v| v as i64),
            _ => Some(off),
        }
        .ok_or(-1)?;

        if new_pos as u64 > (size as _) {
            log::warn!("Seek beyond the end of the block device");
        }
        dev.set_position(new_pos as u64);
        Ok(new_pos)
    }

    fn flush(_dev: &mut Self::DevType) -> Result<usize, i32> {
        todo!()
    }
}



/// 解析 virtio 设备名，如 "/dev/vda", "/dev/vdb" 等，返回其索引。
/// "/dev/vda" -> 0, "/dev/vdb" -> 1, ...
pub fn parse_virtio_device_name(name: &str) -> Option<usize> {
    const PREFIX: &str = "/dev/vd";
    if !name.starts_with(PREFIX) {
        return None;
    }

    // 获取前缀后面的字符
    let suffix = &name[PREFIX.len()..];
    if suffix.len() == 1 {
        let last_char = suffix.chars().next()?;
        if last_char.is_ascii_lowercase() {
            // 计算索引 'a' -> 0, 'b' -> 1, ...
            Some((last_char as u32 - 'a' as u32) as usize)
        } else {
            None
        }
    } else {
        None
    }
}