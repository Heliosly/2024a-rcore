use core::{future::Future, pin::Pin, task::Poll};

use alloc::{boxed::Box, sync::{Arc, Weak}};
use spin::Mutex;

use crate::{mm::UserBuffer, task::yield_now, utils::error::{ASyncRet, ASyscallRet, SysErrNo}};

use super::{File, OpenFlags};

/// IPC pipe
pub struct Pipe {
    #[allow(unused)]
    readable: bool,
    #[allow(unused)]
    writable: bool,
    buffer: Arc<Mutex<PipeRingBuffer>>,
    #[allow(unused)]
    flags: Mutex<OpenFlags>,
}

impl Pipe {
    /// create readable pipe
    pub fn read_end_with_buffer(buffer: Arc<Mutex<PipeRingBuffer>>, flags: OpenFlags) -> Self {
        Self {
            readable: true,
            writable: false,
            buffer,
            flags: Mutex::new(flags | OpenFlags::O_RDONLY),
        }
    }
    /// create writable pipe
    pub fn write_end_with_buffer(buffer: Arc<Mutex<PipeRingBuffer>>, flags: OpenFlags) -> Self {
        Self {
            readable: false,
            writable: true,
            buffer,
            flags: Mutex::new(flags | OpenFlags::O_WRONLY),
        }
    }
    /// is it set non block?
    pub fn is_non_block(&self) -> bool {
        self.flags.lock().contains(OpenFlags::O_NONBLOCK)
    }
}

const RING_BUFFER_SIZE: usize = 0x4000;

#[derive(Copy, Clone, PartialEq)]
enum RingBufferStatus {
    Full,
    Empty,
    Normal,
}

pub struct PipeRingBuffer {
    arr: [u8; RING_BUFFER_SIZE],
    head: usize,
    tail: usize,
    status: RingBufferStatus,
    write_end: Option<Weak<Pipe>>,
}

impl PipeRingBuffer {
    pub fn new() -> Self {
        Self {
            arr: [0; RING_BUFFER_SIZE],
            head: 0,
            tail: 0,
            status: RingBufferStatus::Empty,
            write_end: None,
        }
    }

    pub fn set_write_end(&mut self, write_end: &Arc<Pipe>) {
        self.write_end = Some(Arc::downgrade(write_end));
    }

    pub fn write_byte(&mut self, byte: u8) {
        self.status = RingBufferStatus::Normal;
        self.arr[self.tail] = byte;
        self.tail = (self.tail + 1) % RING_BUFFER_SIZE;
        if self.tail == self.head {
            self.status = RingBufferStatus::Full;
        }
    }
    pub fn read_byte(&mut self) -> u8 {
        self.status = RingBufferStatus::Normal;
        let c = self.arr[self.head];
        self.head = (self.head + 1) % RING_BUFFER_SIZE;
        if self.head == self.tail {
            self.status = RingBufferStatus::Empty;
        }
        c
    }
    pub fn available_read(&self) -> usize {
        if self.status == RingBufferStatus::Empty {
            0
        } else if self.tail > self.head {
            self.tail - self.head
        } else {
            self.tail + RING_BUFFER_SIZE - self.head
        }
    }
    pub fn available_write(&self) -> usize {
        if self.status == RingBufferStatus::Full {
            0
        } else {
            RING_BUFFER_SIZE - self.available_read()
        }
    }
    pub fn all_write_ends_closed(&self) -> bool {
        self.write_end.as_ref().unwrap().upgrade().is_none()
    }
}

/// Return (read_end, write_end)
pub async fn make_pipe(flags: OpenFlags) -> (Arc<Pipe>, Arc<Pipe>) {
    trace!("kernel: make_pipe");
    let buffer = Arc::new(Mutex::new(PipeRingBuffer::new()));
    let read_end = Arc::new(Pipe::read_end_with_buffer(buffer.clone(), flags));
    let write_end = Arc::new(Pipe::write_end_with_buffer(buffer.clone(), flags));
    buffer.lock().set_write_end(&write_end);
    (read_end, write_end)
}
impl File for Pipe{
    fn readable(&self) ->ASyncRet<bool> {
        Box::pin(async { Ok(self.readable) })
    }
    fn writable(&self) -> ASyncRet<bool> {
        Box::pin(async { Ok(self.writable) })
    }
    fn read(&self, mut  buf: UserBuffer) -> ASyscallRet{
       Box::pin(async move {
            assert!(self.readable);
            let want_to_read = buf.len();
            let mut buf_iter = buf.buffers.iter_mut();
            let mut already_read = 0usize;
            loop {
                let mut ring_buffer = self.buffer.lock();
                let loop_read = ring_buffer.available_read();
                info!("kernel: Pipe::read: loop_read = {}", loop_read);
                if loop_read == 0 {
                   
                    info!(
                        "kernel: Pipe::read: all_write_ends_closed = {}",
                        ring_buffer.all_write_ends_closed()
                    );
                    if Arc::strong_count(&self.buffer) < 2 || ring_buffer.all_write_ends_closed() {
                        return Ok(already_read);
                    }

                    if self.is_non_block() {
                        return Err(SysErrNo::EAGAIN);
                    }
                    drop(ring_buffer);
                    yield_now().await;
                    continue;
                }
                for buf in &mut buf_iter {
                    for byte_ref in buf.iter_mut() {
                        *byte_ref = ring_buffer.read_byte();
                        already_read += 1;
                
                        if already_read == want_to_read {
                            return Ok(want_to_read);
                        }
                
                        if already_read == loop_read {
                            return Ok(already_read);
                        }
                    }
                }
            }
       
    }
)
    
}

    fn write(&self, buf: crate::mm::UserBuffer) -> ASyscallRet {
    Box::pin( async move {
            info!("kernel: Pipe::write");
            assert!(self.writable);
            let want_to_write = buf.len();
            let mut buf_iter = buf.buffers.iter();
            let mut already_write = 0usize;
            loop {
                let mut ring_buffer = self.buffer.lock();
                let loop_write = ring_buffer.available_write();
                if loop_write == 0 {
                    drop(ring_buffer);

                    if Arc::strong_count(&self.buffer) < 2 || self.is_non_block() {
                        // 读入端关闭
                        return Ok(already_write);
                    }
                    yield_now().await;
                    continue;
                }

                for buf in &mut buf_iter {
                    for byte_ref in buf.iter() {
                        ring_buffer.write_byte(*byte_ref);
                        already_write += 1;
                
                        if already_write ==want_to_write {
                            return Ok(want_to_write);
                        }
                
                        if already_write == loop_write {
                            return Ok(already_write);
                        }
                    }
                }
               
                return Ok(already_write);
            }
        }
    )  
    }
}