use alloc::sync::Arc;

use crate::utils::error::TemplateRet;

use super::{File, Kstat};

mod simple_net;
pub use simple_net::*;
pub struct Socket;

pub fn make_socket() -> Arc<dyn File> {
    Arc::new(Socket {})
}

impl File for Socket {
    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(false)
      }
       fn readable<'a>(&'a self) -> TemplateRet<bool> {
          unimplemented!();
        }
}
