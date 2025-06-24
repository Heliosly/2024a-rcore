use alloc::sync::Arc;
use async_trait::async_trait;

use crate::{
    mm::UserBuffer,
    utils::error::{SysErrNo, TemplateRet},
};
use alloc::boxed::Box;

use super::{File, Kstat};

mod simple_net;
pub use simple_net::*;
pub struct Socket;

pub fn make_socket() -> Arc<dyn File> {
    Arc::new(Socket {})
}
#[async_trait]
impl File for Socket {
    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(false)
    }

    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>(&self, mut buf: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        if buf.is_empty() {
            return Ok(0);
        }
        buf.write(&[b'1']);
        Ok(1)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
