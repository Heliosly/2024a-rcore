use alloc::sync::Arc;
use lazy_init::LazyInit;


use crate::fs::Statfs;
use spin::Mutex;
use crate::utils::error::{SysErrNo, TemplateRet};
use super::VfsOps;

pub mod ops;





pub static EXT4FS: LazyInit<Arc<Mutex<dyn VfsOps>>> =LazyInit::new();

pub fn fs_stat() -> TemplateRet< Statfs> {
    EXT4FS.lock().statfs().map_err(|e|SysErrNo::from(e) )
}
