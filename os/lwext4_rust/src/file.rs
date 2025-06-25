use core::ffi::c_char;

use crate::bindings::*;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::{ffi::CString, vec::Vec};
use hashbrown::HashSet;
use spin::{Lazy, Mutex, RwLock};

const PAGE_SIZE: usize = 4096;
pub const PAGE_MASK: usize = !0xfff;

fn aligned_down(addr: usize) -> usize {
    addr & PAGE_MASK
}

// Ext4File文件操作与block device设备解耦了
pub struct Ext4File {
    //file_desc_map: BTreeMap<CString, ext4_file>,
    file_desc: ext4_file,
    file_path: CString,

    this_type: InodeTypes,

    has_opened: bool,
    last_flags: u32,
}

impl Ext4File {
    pub fn new(path: &str, types: InodeTypes) -> Self {
        Self {
            file_desc: ext4_file {
                mp: core::ptr::null_mut(),
                inode: 0,
                flags: 0,
                fsize: 0,
                fpos: 0,
            },
            file_path: CString::new(path).expect("CString::new Ext4File path failed"),
            this_type: types,
            has_opened: false,
            last_flags: 0,
        }
    }

    pub fn get_path(&self) -> CString {
        self.file_path.clone()
    }

    pub fn get_type(&self) -> InodeTypes {
        self.this_type.clone()
    }

    /// File open function.
    ///
    /// |---------------------------------------------------------------|
    /// |   r or rb                 O_RDONLY                            |
    /// |---------------------------------------------------------------|
    /// |   w or wb                 O_WRONLY|O_CREAT|O_TRUNC            |
    /// |---------------------------------------------------------------|
    /// |   a or ab                 O_WRONLY|O_CREAT|O_APPEND           |
    /// |---------------------------------------------------------------|
    /// |   r+ or rb+ or r+b        O_RDWR                              |
    /// |---------------------------------------------------------------|
    /// |   w+ or wb+ or w+b        O_RDWR|O_CREAT|O_TRUNC              |
    /// |---------------------------------------------------------------|
    /// |   a+ or ab+ or a+b        O_RDWR|O_CREAT|O_APPEND             |
    /// |---------------------------------------------------------------|
    pub fn file_open(&mut self, path: &str, flags: u32) -> Result<usize, i32> {
        let c_path = CString::new(path).expect("CString::new failed");
        if c_path != self.get_path() {
            // debug!(
            //     "Ext4File file_open, cur path={}, new path={}",
            //     self.file_path.to_str().unwrap(),
            //     path
            // );
        } else {
            if self.has_opened && self.last_flags == flags {
                //如果之前已经按相同方式打开
                //debug!("reopen");
                return Ok(EOK as usize);
            }
        }

        //let to_map = c_path.clone();
        let c_path = c_path.into_raw();
        let c_flags = Self::flags_to_cstring(flags);
        let c_flags = c_flags.into_raw();

        let r = unsafe { ext4_fopen(&mut self.file_desc, c_path, c_flags) };
        unsafe {
            // deallocate the CString
            drop(CString::from_raw(c_path));
            drop(CString::from_raw(c_flags));
        }
        if r != EOK as i32 {
            error!("ext4_fopen: {}, rc = {}", path, r);
            return Err(r);
        }

        self.has_opened = true;
        self.last_flags = flags;

        //self.file_desc_map.insert(to_map, fd); // store c_path
        //debug!("file_open {}, mp={:#x}", path, self.file_desc.mp as usize);
        Ok(EOK as usize)
    }

    pub fn file_close(&mut self) -> Result<usize, i32> {
        if self.file_desc.mp != core::ptr::null_mut() {
            //debug!("file_close {:?}", self.get_path());
            // self.file_cache_flush()?;
            unsafe {
                ext4_fclose(&mut self.file_desc);
            }
        }

        self.has_opened = false;

        Ok(0)
    }

    pub fn flags_to_cstring(flags: u32) -> CString {
        let cstr = match flags {
            O_RDONLY => "rb",
            O_RDWR => "r+",
            0x241 => "wb", // O_WRONLY | O_CREAT | O_TRUNC
            0x441 => "ab", // O_WRONLY | O_CREAT | O_APPEND
            0x242 => "w+", // O_RDWR | O_CREAT | O_TRUNC
            0x442 => "a+", // O_RDWR | O_CREAT | O_APPEND
            _ => {
                warn!("Unknown File Open Flags: {:#x}", flags);
                "r+"
            }
        };
        //debug!("flags_to_cstring: {}", cstr);
        CString::new(cstr).expect("CString::new OpenFlags failed")
    }

    /// Inode types:
    /// EXT4_DIRENTRY_UNKNOWN
    /// EXT4_DE_REG_FILE
    /// EXT4_DE_DIR
    /// EXT4_DE_CHRDEV
    /// EXT4_DE_BLKDEV
    /// EXT4_DE_FIFO
    /// EXT4_DE_SOCK
    /// EXT4_DE_SYMLINK
    ///
    /// Check if inode exists.
    pub fn check_inode_exist(&mut self, path: &str, types: InodeTypes) -> bool {
        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();
        let mtype = types.clone();
        let r = unsafe { ext4_inode_exist(c_path, types as i32) }; //eg: types: EXT4_DE_REG_FILE
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r == EOK as i32 {
            // debug!("{:?} {} Exist", mtype, path);
            true //Exist
        } else {
            debug!("{:?} {} No Exist. ext4_inode_exist rc = {}", mtype, path, r);
            false
        }
    }

    pub fn file_readlink(&mut self, buf: &mut [u8], bufsize: usize) -> Result<usize, i32> {
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let buf_c_char = buf.as_mut_ptr() as *mut c_char;
        let mut rcnt = 0usize;
        let r = unsafe { ext4_readlink(c_path, buf_c_char, bufsize, &mut rcnt) };
        if r != EOK as i32 {
            error!("ext4_readlink error: rc = {}", r);
            return Err(r);
        }
        Ok(rcnt)
    }

    //ext4_fsymlink(const char *target, const char *path)
    pub fn file_fsymlink(&mut self, target: &str, path: &str) -> Result<usize, i32> {
        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();
        let target_path = CString::new(target).expect("CString::new failed");
        let target_path = target_path.into_raw();
        let r = unsafe { ext4_fsymlink(target_path, c_path) };
        if r != EOK as i32 {
            error!("ext4_fsymlink error: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    /// Rename file and directory
    pub fn file_rename(&mut self, path: &str, new_path: &str) -> Result<usize, i32> {
        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();
        let c_new_path = CString::new(new_path).expect("CString::new failed");
        let c_new_path = c_new_path.into_raw();
        let r = unsafe { ext4_frename(c_path, c_new_path) };
        unsafe {
            drop(CString::from_raw(c_path));
            drop(CString::from_raw(c_new_path));
        }
        if r != EOK as i32 {
            error!("ext4_frename error: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    /// Remove file by path.
    pub fn file_remove(&mut self, path: &str) -> Result<usize, i32> {
        //debug!("file_remove {}", path);

        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();

        let path = String::from(path);
        //删掉对应缓存
        if if_cache(path.clone()) {
            remove_cache(path);
        }

        //修改为未打开
        self.has_opened = false;

        let r = unsafe { ext4_fremove(c_path) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if (r != EOK as i32) && (r != ENOENT as i32) {
            error!("ext4_fremove error: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    //检查是否在cache表中，没有则添加
    fn check_cached(&mut self, file_path: String) {
        if !if_cache(file_path.clone()) {
            insert_fifo(file_path.clone());
            let cache = Arc::new(RwLock::new(VFileCache::new()));
            let mut cache_writer = cache.write();
            debug!("initialize cache! {}", file_path);
            let c_path = CString::new(file_path.as_str()).expect("CString::new failed");
            let c_path = c_path.into_raw();
            let c_flags = Ext4File::flags_to_cstring(2).into_raw();

            //重新打开文件获得最新的文件信息
            unsafe { ext4_fopen(&mut self.file_desc, c_path, c_flags) };
            unsafe {
                // deallocate the CString
                drop(CString::from_raw(c_path));
                drop(CString::from_raw(c_flags));
            }

            let size = unsafe { ext4_fsize(&mut self.file_desc) as usize };
            //debug!("initialize size={}", size);
            let aligned_size = aligned_down(size) + PAGE_SIZE;
            cache_writer.data = Vec::with_capacity(aligned_size);
            let data = &mut cache_writer.data;
            unsafe {
                data.set_len(aligned_size);
            }
            cache_writer.size = size;
            if size == 0 {
                insert_cache(file_path.clone(), &cache);
                return;
            }
            unsafe { ext4_fseek(&mut self.file_desc, 0, 0) };
            let mut rw_count = 0;
            unsafe {
                ext4_fread(
                    &mut self.file_desc,
                    cache_writer.data.as_mut_ptr() as _,
                    size,
                    &mut rw_count,
                )
            };
            insert_cache(file_path.clone(), &cache);
        }
    }

    pub fn file_seek(&mut self, offset: i64, seek_type: u32) -> Result<usize, i32> {
        if self.this_type != InodeTypes::EXT4_DE_DIR {
            //如果是目录文件不用cache
            let path = String::from((*self.file_path).to_str().unwrap());
            self.check_cached(path.clone());

            let cache = get_cache(path.clone());
            let mut cache_writer = cache.write();

            let offset = offset as usize;
            if offset > cache_writer.size {
                /*
                warn!(
                    "Seek beyond the end of the file,path is {},offset is {} while size is {}",
                    path, offset, cache_writer.size
                );
                */
                //offset = cache_writer.size;
                cache_writer.truncate(offset);
            }

            cache_writer.offset = offset as usize;
            //debug!("offset change to {:x}", offset);
            return Ok(EOK as usize);
        }

        let mut offset = offset;
        let size = self.file_size() as i64;

        if offset > size {
            warn!("Seek beyond the end of the file");
            offset = size;
        }

        let r = unsafe { ext4_fseek(&mut self.file_desc, offset, seek_type) };
        if r != EOK as i32 {
            error!("ext4_fseek: rc = {}", r);
            return Err(r);
        }

        Ok(r as usize)
    }

    pub fn file_read(&mut self, buff: &mut [u8]) -> Result<usize, i32> {
        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            //找到cache直接读cache
            let cache = get_cache(path.clone());
            let cache_read = cache.read();
            let data = cache_read.get_data_slice();
            let length = buff.len();
            let end = (cache_read.offset + length).min(cache_read.size);
            let r_sz = end - cache_read.offset;
            //debug!("data.len={:x},end={:x}", data.len(), end);
            if length <= 10 {
                for i in 0..r_sz {
                    buff[i] = data[cache_read.offset + i];
                }
            } else {
                buff[..r_sz].copy_from_slice(&data[cache_read.offset..end]);
            }

            /*
            debug!(
                "file_read {},len = {:x},offset is {:x}",
                path, r_sz, cache_read.offset
            );
            */

            return Ok(r_sz);
        }

        let mut rw_count = 0;
        let r = unsafe {
            ext4_fread(
                &mut self.file_desc,
                buff.as_mut_ptr() as _,
                buff.len(),
                &mut rw_count,
            )
        };

        if r != EOK as i32 {
            error!("ext4_fread: rc = {}", r);
            return Err(r);
        }

        //debug!("file_read {:?}, len={}", self.get_path(), rw_count);
        Ok(rw_count)
    }

    /*
    pub fn file_close(&mut self, path: &str) -> Result<usize, i32> {
        let cstr_path = CString::new(path).unwrap();
        if let Some(mut fd) = self.file_desc_map.remove(&cstr_path) {
            unsafe {
                ext4_fclose(&mut fd);
            }
            Ok(0)
        } else {
            error!("Can't find file descriptor of {}", path);
            Err(-1)
        }
    }
    */

    pub fn file_write(&mut self, buf: &[u8]) -> Result<usize, i32> {
        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            //找到cache直接写cache
            let cache = get_cache(path.clone());
            let mut cache_writer = cache.write();
            #[allow(unused)]
            let len = cache_writer.writebuf(buf);

            /*
            debug!(
                "file write at {} with size={} offset={}",
                path,
                buf.len(),
                cache_writer.offset
            );
            */

            //debug!("len is {} now", len);
            #[cfg(feature = "board_qemu")]
            if len > 5_100_000 {
                log::info!("len is {} out of mem", len);
                //write_back_cache(path.clone());
                remove_cache(path.clone());
                return Err(ENOMEM as i32);
            }
            return Ok(buf.len());
        }

        let mut rw_count = 0;
        let r = unsafe {
            ext4_fwrite(
                &mut self.file_desc,
                buf.as_ptr() as _,
                buf.len(),
                &mut rw_count,
            )
        };

        if r != EOK as i32 {
            error!("ext4_fwrite: rc = {}", r);
            return Err(r);
        }

        //debug!("file_write {:?}, len={}", self.get_path(), rw_count);
        Ok(rw_count)
    }

    pub fn file_truncate(&mut self, size: u64) -> Result<usize, i32> {
        debug!("file_truncate to {}", size);

        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            let cache = get_cache(path.clone());
            let mut cache_writer = cache.write();
            cache_writer.truncate(size as usize);
        }

        let r = unsafe { ext4_ftruncate(&mut self.file_desc, size) };
        if r != EOK as i32 {
            error!("ext4_ftruncate: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    pub fn file_size(&mut self) -> u64 {
        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            return get_cache(path.clone()).read().size as u64;
        }

        //注，记得先 O_RDONLY 打开文件
        let c_path = self.file_path.clone().into_raw();
        let c_flags = Ext4File::flags_to_cstring(2).into_raw();

        //重新打开文件获得最新的文件信息
        unsafe { ext4_fopen(&mut self.file_desc, c_path, c_flags) };
        unsafe {
            // deallocate the CString
            drop(CString::from_raw(c_path));
            drop(CString::from_raw(c_flags));
        }
        unsafe { ext4_fsize(&mut self.file_desc) }
    }

    pub fn file_cache_flush(&mut self) -> Result<usize, i32> {
        /*
        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            write_back_cache(path.clone());
        }
        */

        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        unsafe {
            let r = ext4_cache_flush(c_path);
            if r != EOK as i32 {
                error!("ext4_cache_flush: rc = {}", r);
                return Err(r);
            }
            drop(CString::from_raw(c_path));
        }
        Ok(0)
    }

    //int ext4_owner_set(const char *path, uint32_t uid, uint32_t gid)
    pub fn set_owner(&mut self, uid: u32, gid: u32) -> Result<usize, i32> {
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();

        let r = unsafe { ext4_owner_set(c_path, uid, gid) };

        // unsafe { ext4_mode_set(c_path, mode) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_owner_set: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    pub fn set_time(
        &mut self,
        atime: Option<u32>,
        mtime: Option<u32>,
        ctime: Option<u32>,
    ) -> Result<usize, i32> {
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let mut r = 0;
        if let Some(atime) = atime {
            r = unsafe { ext4_atime_set(c_path, atime) }
        }
        if let Some(mtime) = mtime {
            r = unsafe { ext4_mtime_set(c_path, mtime) }
        }
        if let Some(ctime) = ctime {
            r = unsafe { ext4_ctime_set(c_path, ctime) }
        }
        // unsafe { ext4_mode_set(c_path, mode) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_time_set: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }
    // Ok(atime,mtime,ctime)
    // pub fn time(&mut self) -> Result<(u32, u32, u32), i32> {
    //     let (mut atime, mut mtime, mut ctime) = (0, 0, 0);
    //     let c_path = self.file_path.clone();
    //     let c_path = c_path.into_raw();
    //     let r = unsafe {
    //         ext4_atime_get(c_path, &mut atime)
    //             | ext4_mtime_get(c_path, &mut mtime)
    //             | ext4_ctime_get(c_path, &mut ctime)
    //     };
    //     unsafe {
    //         drop(CString::from_raw(c_path));
    //     }
    //     if r != EOK as i32 {
    //         error!("ext4_mode_get: rc = {}", r);
    //         return Err(r);
    //     }
    //     Ok((atime, mtime, ctime))
    // }
    pub fn fstat(&mut self) -> Result<ext4_inode_stat, i32> {
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let mut stat = ext4_inode_stat::default();
        let r = unsafe { ext4_stat_get(c_path, &mut stat) };

        let path = String::from((*self.file_path).to_str().unwrap());
        if if_cache(path.clone()) {
            //如果在缓存中，更新stat获得的大小
            let cache = get_cache(path.clone());
            let cache_reader = cache.read();
            stat.st_size = cache_reader.size as isize;
            stat.st_blocks =
                (stat.st_size - 1 + (stat.st_blksize as isize)) / (stat.st_blksize as isize);
        }

        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_stat_get: rc = {}", r);
            return Err(r);
        }
        Ok(stat)
    }

    pub fn links_cnt(&mut self) -> Result<u32, i32> {
        let mut cnt: u32 = 0;
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let r = unsafe { ext4_get_links_cnt(c_path, &mut cnt) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            // error!("ext4_links_cnt_get: rc = {}", r);
            return Err(r);
        }
        Ok(cnt)
    }
    pub fn file_mode(&mut self) -> Result<u32, i32> {
        // 0o777 (octal) == rwxrwxrwx
        let mut mode: u32 = 0o777;
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let r = unsafe { ext4_mode_get(c_path, &mut mode) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_mode_get: rc = {}", r);
            return Err(r);
        }
        //debug!("Got file mode={:#x}", mode);
        Ok(mode)
    }

    pub fn file_mode_set(&mut self, mode: u32) -> Result<usize, i32> {
        //debug!("file_mode_set to {:#x}", mode);

        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let r = unsafe { ext4_mode_set(c_path, mode) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_mode_set: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    pub fn file_type(&mut self) -> InodeTypes {
        let mode = self.file_mode().unwrap();
        // 0o777 (octal) == rwxrwxrwx
        // if filetype == EXT4_DE_SYMLINK;
        // mode = 0777;
        // mode |= EXT4_INODE_MODE_SOFTLINK;
        let cal: u32 = 0o777;
        let types = mode & (!cal);
        let itypes = match types {
            0x1000 => InodeTypes::EXT4_INODE_MODE_FIFO,
            0x2000 => InodeTypes::EXT4_INODE_MODE_CHARDEV,
            0x4000 => InodeTypes::EXT4_INODE_MODE_DIRECTORY,
            0x6000 => InodeTypes::EXT4_INODE_MODE_BLOCKDEV,
            0x8000 => InodeTypes::EXT4_INODE_MODE_FILE,
            0xA000 => InodeTypes::EXT4_INODE_MODE_SOFTLINK,
            0xC000 => InodeTypes::EXT4_INODE_MODE_SOCKET,
            0xF000 => InodeTypes::EXT4_INODE_MODE_TYPE_MASK,
            _ => {
                warn!("Unknown inode mode type {:x}", types);
                InodeTypes::EXT4_INODE_MODE_FILE
            }
        };
        //debug!("Inode mode types: {:?}", itypes);

        itypes
    }

    /********* DIRECTORY OPERATION *********/

    /// Create new directory
    pub fn dir_mk(&mut self, path: &str) -> Result<usize, i32> {
        //debug!("directory create: {}", path);
        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();

        let r = unsafe { ext4_dir_mk(c_path) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if r != EOK as i32 {
            error!("ext4_dir_mk: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    /// Rename/move directory
    pub fn dir_mv(&mut self, path: &str, new_path: &str) -> Result<usize, i32> {
        //debug!("directory move from {} to {}", path, new_path);

        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();
        let c_new_path = CString::new(new_path).expect("CString::new failed");
        let c_new_path = c_new_path.into_raw();

        let r = unsafe { ext4_dir_mv(c_path, c_new_path) };
        unsafe {
            drop(CString::from_raw(c_path));
            drop(CString::from_raw(c_new_path));
        }
        if r != EOK as i32 {
            error!("ext4_dir_mv: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    /// Recursive directory remove
    pub fn dir_rm(&mut self, path: &str) -> Result<usize, i32> {
        //debug!("directory recursive remove: {}", path);

        let c_path = CString::new(path).expect("CString::new failed");
        let c_path = c_path.into_raw();

        let r = unsafe { ext4_dir_rm(c_path) };
        unsafe {
            drop(CString::from_raw(c_path));
        }
        if (r != EOK as i32) && (r != ENOENT as i32) {
            error!("ext4_fremove ext4_dir_rm: rc = {}", r);
            return Err(r);
        }
        Ok(EOK as usize)
    }

    ///如果读完返回-1
    pub fn read_dir_from(&self, off: u64) -> Result<Vec<OsDirent>, i32> {
        if self.this_type != InodeTypes::EXT4_DE_DIR {
            return Err(22);
        }
        let c_path = self.file_path.clone();
        let c_path = c_path.into_raw();
        let mut d: ext4_dir = unsafe { core::mem::zeroed() };
        let mut entries: Vec<_> = Vec::new();

        unsafe {
            ext4_dir_open(&mut d, c_path);
            drop(CString::from_raw(c_path));
            d.next_off = off;
            let mut de = ext4_dir_entry_next(&mut d);
            while !de.is_null() {
                let dentry = &(*de);
                //对齐 align8
                let mut name = [0u8; 256];
                let name_len = dentry.name_length as usize;
                name[0..name_len].copy_from_slice(&dentry.name[0..name_len]);
                let mut len = name_len + 19;
                let align = 8 - len % 8;
                len += align;
                entries.push(OsDirent {
                    d_ino: dentry.inode as u64,
                    d_off: d.next_off as i64,
                    d_reclen: len as u16,
                    d_type: dentry.inode_type,
                    d_name: name,
                });
                de = ext4_dir_entry_next(&mut d);
            }
            ext4_dir_close(&mut d);
        }
        Ok(entries)
    }
}

/*
pub enum OpenFlags {
O_RDONLY = 0,
O_WRONLY = 0x1,
O_RDWR = 0x2,
O_CREAT = 0x40,
O_TRUNC = 0x200,
O_APPEND = 0x400,
}
*/

#[derive(PartialEq, Clone, Debug)]
pub enum InodeTypes {
    // Inode type, Directory entry types.
    EXT4_DE_UNKNOWN = 0,
    EXT4_DE_REG_FILE = 1,
    EXT4_DE_DIR = 2,
    EXT4_DE_CHRDEV = 3,
    EXT4_DE_BLKDEV = 4,
    EXT4_DE_FIFO = 5,
    EXT4_DE_SOCK = 6,
    EXT4_DE_SYMLINK = 7,

    // Inode mode
    EXT4_INODE_MODE_FIFO = 0x1000,
    EXT4_INODE_MODE_CHARDEV = 0x2000,
    EXT4_INODE_MODE_DIRECTORY = 0x4000,
    EXT4_INODE_MODE_BLOCKDEV = 0x6000,
    EXT4_INODE_MODE_FILE = 0x8000,
    EXT4_INODE_MODE_SOFTLINK = 0xA000,
    EXT4_INODE_MODE_SOCKET = 0xC000,
    EXT4_INODE_MODE_TYPE_MASK = 0xF000,
}

impl From<usize> for InodeTypes {
    fn from(num: usize) -> InodeTypes {
        match num {
            0 => InodeTypes::EXT4_DE_UNKNOWN,
            1 => InodeTypes::EXT4_DE_REG_FILE,
            2 => InodeTypes::EXT4_DE_DIR,
            3 => InodeTypes::EXT4_DE_CHRDEV,
            4 => InodeTypes::EXT4_DE_BLKDEV,
            5 => InodeTypes::EXT4_DE_FIFO,
            6 => InodeTypes::EXT4_DE_SOCK,
            7 => InodeTypes::EXT4_DE_SYMLINK,
            0x1000 => InodeTypes::EXT4_INODE_MODE_FIFO,
            0x2000 => InodeTypes::EXT4_INODE_MODE_CHARDEV,
            0x4000 => InodeTypes::EXT4_INODE_MODE_DIRECTORY,
            0x6000 => InodeTypes::EXT4_INODE_MODE_BLOCKDEV,
            0x8000 => InodeTypes::EXT4_INODE_MODE_FILE,
            0xA000 => InodeTypes::EXT4_INODE_MODE_SOFTLINK,
            0xC000 => InodeTypes::EXT4_INODE_MODE_SOCKET,
            0xF000 => InodeTypes::EXT4_INODE_MODE_TYPE_MASK,
            _ => {
                warn!("Unknown ext4 inode type: {}", num);
                InodeTypes::EXT4_DE_UNKNOWN
            }
        }
    }
}
#[repr(C)]
#[derive(Debug)]
pub struct OsDirent {
    pub d_ino: u64,        // 索引节点号
    pub d_off: i64,        // 从 0 开始到下一个 dirent 的偏移
    pub d_reclen: u16,     // 当前 dirent 的长度
    pub d_type: u8,        // 文件类型
    pub d_name: [u8; 256], // 文件名
}

#[derive(Clone)]
pub struct VFileCache {
    data: Vec<u8>,
    offset: usize,
    modified: bool,
    size: usize,
}

impl VFileCache {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            offset: 0,
            modified: false,
            size: 0,
        }
    }

    pub fn get_data_slice(&self) -> &[u8] {
        &self.data.as_slice()[..]
    }

    pub fn writebuf(&mut self, buf: &[u8]) -> usize {
        let length = buf.len();
        if self.offset + length > self.size {
            self.size = self.offset + length;
        }
        if self.offset + length > self.data.len() {
            let aligned_size = aligned_down(self.offset + length) + PAGE_SIZE;
            self.data.resize(aligned_size, 0);
        }
        if length <= 10 {
            for i in 0..length {
                self.data[self.offset + i] = buf[i];
            }
        } else {
            self.data[self.offset..self.offset + length].copy_from_slice(buf);
        }
        self.modified = true;
        /*
        debug!(
            "write {} bytes and size is {}, data.len() is {} now",
            length,
            self.size,
            self.data.len()
        );
        */
        return self.data.len();
    }

    pub fn truncate(&mut self, new_size: usize) {
        let aligned_size = aligned_down(new_size) + PAGE_SIZE;
        self.data.resize(aligned_size, 0);
        let length = self.data.len();
        if new_size < length {
            self.data[new_size..length].fill(0);
        }
        self.size = new_size;
    }
}

//cache表，目前只为非目录文件使用cache
static CACHE_TABLE: Lazy<Mutex<BTreeMap<String, Arc<RwLock<VFileCache>>>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

pub fn if_cache(file_path: String) -> bool {
    CACHE_TABLE.lock().contains_key(&file_path)
}

pub fn get_cache(file_path: String) -> Arc<RwLock<VFileCache>> {
    CACHE_TABLE.lock().get(&file_path).unwrap().clone()
}

pub fn insert_cache(file_path: String, cache: &Arc<RwLock<VFileCache>>) {
    CACHE_TABLE.lock().insert(file_path, cache.clone());
}

pub fn remove_cache(file_path: String) {
    CACHE_TABLE.lock().remove(&file_path);
}

const FIFO_SIZE: usize = 30;
//采用先进先出策略
static FIFO_TABLE: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

pub fn insert_fifo(file_path: String) {
    let mut fifo = FIFO_TABLE.lock();
    //队列中存在该文件，说明之前被删除过，不重复加入
    if if_fifo_set(file_path.clone()) {
        // debug!("file {} already exist", file_path);
        return;
    }
    if fifo.len() == FIFO_SIZE {
        //替换并可能写回
        let path = fifo.pop_front().unwrap();
        write_back_cache(path.clone());
        if if_cache(path.clone()) {
            remove_cache(path.clone());
            remove_fifo_set(path.clone());
        }
        // debug!("\n\n{} is replaced!\n\n", path);
    }
    fifo.push_back(file_path.clone());
    insert_fifo_set(file_path.clone());
    // debug!(
    //     "\n\ninsert {} into fifo!\nlen is {}\n\n",
    //     file_path,
    //     fifo.len()
    // );
}

static FIFO_SET: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub fn insert_fifo_set(file_path: String) {
    FIFO_SET.lock().insert(file_path);
}

pub fn if_fifo_set(file_path: String) -> bool {
    FIFO_SET.lock().contains(&file_path)
}

pub fn remove_fifo_set(file_path: String) {
    FIFO_SET.lock().remove(&file_path);
}

pub fn write_back_cache(path: String) {
    if if_cache(path.clone()) {
        //如果在缓存中有，表明未被删除
        let cache = get_cache(path.clone());
        let cache_writer = cache.write();
        if cache_writer.modified {
            //如果被修改过，则写回
            // debug!("{} is written back!", path);
            let c_path = CString::new(path.as_str()).expect("CString::new failed");
            let c_path = c_path.into_raw();
            let flags = Ext4File::flags_to_cstring(2).into_raw();
            let mut file_desc = ext4_file {
                mp: core::ptr::null_mut(),
                inode: 0,
                flags: 0,
                fsize: 0,
                fpos: 0,
            };
            unsafe { ext4_fopen(&mut file_desc, c_path, flags) };
            unsafe {
                // deallocate the CString
                drop(CString::from_raw(c_path));
                drop(CString::from_raw(flags));
            }
            unsafe { ext4_fseek(&mut file_desc, 0, 0) };
            let mut rw_count = 0;
            unsafe {
                ext4_fwrite(
                    &mut file_desc,
                    cache_writer.data.as_ptr() as _,
                    cache_writer.size,
                    &mut rw_count,
                )
            };
            unsafe {
                ext4_fclose(&mut file_desc);
            }
        }
    }
}
