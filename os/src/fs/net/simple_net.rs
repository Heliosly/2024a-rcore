use core::task::Waker;

use alloc::sync::Arc;
use async_trait::async_trait;

use super::{File, Kstat};
use crate::{

    fs::{pipe::{make_pipe, Pipe}, stat::StMode, OpenFlags, PollEvents}, mm::UserBuffer, utils::error::{SysErrNo, SyscallRet, TemplateRet}
};
use alloc::boxed::Box;
pub struct SimpleSocket {
    read_end: Arc<Pipe>,
    write_end: Arc<Pipe>,
}

impl SimpleSocket {
    pub fn new(r_end: Arc<Pipe>, w_end: Arc<Pipe>) -> Self {
        Self {
            read_end: r_end,
            write_end: w_end,
        }
    }
}

pub fn make_socketpair(flags:OpenFlags) -> (Arc<SimpleSocket>, Arc<SimpleSocket>) {
    let (r1, w1) = make_pipe(flags);
    let (r2, w2) = make_pipe(flags);
    let socket1 = Arc::new(SimpleSocket::new(r1, w2));
    let socket2 = Arc::new(SimpleSocket::new(r2, w1));
    (socket1, socket2)
}
#[async_trait]
impl File for SimpleSocket {
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(true)
    }

    async fn read<'a>( 
        &self,                 
        mut buf: UserBuffer<'a>
    ) -> SyscallRet {
        Ok(self.read_end.read(buf).await?)
    }

    async fn write<'a>(
        &self,
        buf: UserBuffer<'a>
    ) -> Result<usize, SysErrNo> {
        Ok(self.write_end.write(buf).await?)
    }

    fn fstat(&self) -> Kstat {
        Kstat {
            st_mode: StMode::FSOCK.bits(),
            st_nlink: 1,
            ..Kstat::default()
        }
    }

    fn poll(&self, events: PollEvents, waker_to_register: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        let read_buffer = self.read_end.buffer.lock();
        let write_buffer = self.write_end.buffer.lock();
        if events.contains(PollEvents::POLLIN) && read_buffer.available_read() > 0 {
            revents |= PollEvents::POLLIN;
        }
        if events.contains(PollEvents::POLLOUT) && write_buffer.available_write() > 0 {
            revents |= PollEvents::POLLOUT;
        }
        revents
    }
}
