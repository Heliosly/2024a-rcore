use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use async_trait::async_trait; 
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use spin::Mutex;

use crate::{
    fs::{stat::StMode, File, Kstat, OpenFlags, PollEvents }, // 假设 SeekWhence 在这里
    mm::UserBuffer,
    task::yield_now, 
    utils::error::{SysErrNo, TemplateRet},
};

const RING_BUFFER_SIZE: usize = 0x4000; // 16KB

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RingBufferStatus {
    Full,
    Empty,
    Normal,
}

pub struct PipeRingBuffer {
    arr: [u8; RING_BUFFER_SIZE], 
    head: usize,
    tail: usize,
    status: RingBufferStatus,
    read_end_closed: bool,  
    write_end_closed: bool, 
    read_waiters: Vec<Waker>,  
    write_waiters: Vec<Waker>,
}

impl PipeRingBuffer {
    pub fn new() -> Self {
        Self {
            arr: [0; RING_BUFFER_SIZE],
            head: 0,
            tail: 0,
            status: RingBufferStatus::Empty,
            read_end_closed: false,
            write_end_closed: false,
            read_waiters: Vec::new(),
            write_waiters: Vec::new(),
        }
    }

    /// 辅助函数：唤醒所有等待读的 Waker
    fn notify_readers(&mut self) {
        for waker in self.read_waiters.drain(..) {
            waker.wake();
        }
    }

    /// 辅助函数：唤醒所有等待写的 Waker
    fn notify_writers(&mut self) {
        for waker in self.write_waiters.drain(..) {
            waker.wake();
        }
    }

    /// 辅助函数：注册 Waker 到读等待列表 (带去重/更新)
    fn register_reader_waker(&mut self, new_waker: &Waker) {
        if !self.read_waiters.iter().any(|w| w.will_wake(new_waker)) {
            self.read_waiters.push(new_waker.clone());
        }
        // 更复杂的可以替换：
        // self.read_waiters.retain(|w| !w.will_wake(new_waker));
        // self.read_waiters.push(new_waker.clone());
    }

    /// 辅助函数：注册 Waker 到写等待列表 (带去重/更新)
    fn register_writer_waker(&mut self, new_waker: &Waker) {
        if !self.write_waiters.iter().any(|w| w.will_wake(new_waker)) {
            self.write_waiters.push(new_waker.clone());
        }
    }

    // 你之前的 set_write_end 可以改为更通用的 close 通知
    pub fn mark_write_end_closed(&mut self) {
        if !self.write_end_closed {
            self.write_end_closed = true;
            self.notify_readers(); // 写端关闭，读者可能需要被唤醒以读取EOF
        }
    }

    pub fn mark_read_end_closed(&mut self) {
        if !self.read_end_closed {
            self.read_end_closed = true;
            self.notify_writers(); // 读端关闭，写者可能需要被唤醒以处理EPIPE
        }
    }


    pub fn write_byte(&mut self, byte: u8) -> Result<(), SysErrNo> {
        if self.status == RingBufferStatus::Full {
            return Err(SysErrNo::EAGAIN); // 缓冲区满
        }
        if self.read_end_closed { // 如果读端已关闭
            return Err(SysErrNo::EPIPE); // Broken pipe
        }

        let buffer_was_empty = self.status == RingBufferStatus::Empty;
        self.arr[self.tail] = byte;
        self.tail = (self.tail + 1) % RING_BUFFER_SIZE;
        self.status = if self.tail == self.head {
            RingBufferStatus::Full
        } else {
            RingBufferStatus::Normal
        };

        if buffer_was_empty { // 从空变为非空，通知读者
            self.notify_readers();
        }
        Ok(())
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        if self.status == RingBufferStatus::Empty {
            return None;
        }

        let buffer_was_full = self.status == RingBufferStatus::Full;
        let c = self.arr[self.head];
        self.head = (self.head + 1) % RING_BUFFER_SIZE;
        self.status = if self.head == self.tail {
            RingBufferStatus::Empty
        } else {
            RingBufferStatus::Normal
        };

        if buffer_was_full { // 从满变为不满，通知写者
            self.notify_writers();
        }
        Some(c)
    }

    pub fn available_read(&self) -> usize {
        if self.status == RingBufferStatus::Empty { 0 }
        else if self.tail > self.head { self.tail - self.head }
        else { self.tail + RING_BUFFER_SIZE - self.head }
    }

    pub fn available_write(&self) -> usize {
        if self.status == RingBufferStatus::Full { 0 }
        else { RING_BUFFER_SIZE - self.available_read() }
    }

    // all_write_ends_closed 现在由 write_end_closed 标志管理
}

/// IPC 管道
pub struct Pipe {
    readable: bool,
    writable: bool,
    pub buffer: Arc<Mutex<PipeRingBuffer>>, // 共享的环形缓冲区
    flags: Mutex<OpenFlags>,          // Pipe 自身的打开标志 (如 O_NONBLOCK)
}

impl Pipe {
    pub fn read_end_with_buffer(buffer: Arc<Mutex<PipeRingBuffer>>, flags: OpenFlags) -> Self {
        Self {
            readable: true, writable: false, buffer,
            flags: Mutex::new(flags | OpenFlags::O_RDONLY),
        }
    }
    pub fn write_end_with_buffer(buffer: Arc<Mutex<PipeRingBuffer>>, flags: OpenFlags) -> Self {
        Self {
            readable: false, writable: true, buffer,
            flags: Mutex::new(flags | OpenFlags::O_WRONLY),
        }
    }
    pub fn is_non_block(&self) -> bool {
        self.flags.lock().contains(OpenFlags::O_NONBLOCK)
    }
}

/// 创建管道对 (read_end, write_end)
pub fn make_pipe(flags: OpenFlags) -> (Arc<Pipe>, Arc<Pipe>) {
    // log::trace!("kernel: make_pipe with flags: {:?}", flags);
    let buffer = Arc::new(Mutex::new(PipeRingBuffer::new()));
    // Pipe 的 flags 通常指 O_NONBLOCK, O_CLOEXEC 等，应用于两个 FD。
    // O_RDONLY/O_WRONLY 是 FD 的属性，而不是管道本身的。
    // 但这里为了方便，Pipe 结构也存储了组合后的 flags。
    let read_flags = flags | OpenFlags::O_RDONLY; // 确保读端是只读
    let write_flags = flags | OpenFlags::O_WRONLY; // 确保写端是只写

    let read_end = Arc::new(Pipe::read_end_with_buffer(buffer.clone(), read_flags));
    let write_end = Arc::new(Pipe::write_end_with_buffer(buffer.clone(), write_flags));

    // PipeRingBuffer 不需要知道 Pipe 的 Arc 了，它通过 closed 标志和 Waker 工作。
    // buffer.lock().set_write_end(&write_end); // 这行可以移除，用 closed 标志代替
    (read_end, write_end)
}

// Pipe 的 Drop 实现，用于标记对端关闭
impl Drop for Pipe {
    fn drop(&mut self) {
        let mut buffer_guard = self.buffer.lock();
        if self.readable { // 如果这是读端被 drop
            // log::trace!("Pipe read_end dropped. Notifying writers.");
            buffer_guard.mark_read_end_closed();
        }
        if self.writable { // 如果这是写端被 drop
            // log::trace!("Pipe write_end dropped. Notifying readers.");
            buffer_guard.mark_write_end_closed();
        }
    }
}

// --- Pipe 的 File trait 实现 ---
#[async_trait]
impl File for Pipe {
    fn poll(&self, requested_events: PollEvents, waker_to_register: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        let mut buffer_guard = self.buffer.lock(); // 获取共享缓冲区的锁

        if self.readable { // 如果是管道的读端
            if requested_events.contains(PollEvents::POLLIN) {
                if buffer_guard.available_read() > 0 {
                    revents.insert(PollEvents::POLLIN); // 有数据可读
                } else if buffer_guard.write_end_closed {
                    // 写端已关闭，可以读到 EOF (0字节)，也算可读事件
                    revents.insert(PollEvents::POLLIN);
                    revents.insert(PollEvents::POLLHUP); // 通常伴随 HUP
                } else {
                    // 缓冲区为空，且写端未关闭，注册 Waker 等待数据
                    buffer_guard.register_reader_waker(waker_to_register);
                }
            }
            // 如果写端关闭，对于读端来说是 POLLHUP
            if buffer_guard.write_end_closed {
                revents.insert(PollEvents::POLLHUP);
            }
        }

        if self.writable { // 如果是管道的写端
            if requested_events.contains(PollEvents::POLLOUT) {
                if buffer_guard.available_write() > 0 {
                    revents.insert(PollEvents::POLLOUT); // 有空间可写
                } else if buffer_guard.read_end_closed {
                    // 读端已关闭，写入会导致 EPIPE，算是一种“可操作”的错误状态
                    revents.insert(PollEvents::POLLOUT); // 也可以是 POLLERR
                    revents.insert(PollEvents::POLLERR); // 写入会失败
                    // revents.insert(PollEvents::POLLHUP); // 有些系统也可能报告HUP
                } else {
                    // 缓冲区已满，且读端未关闭，注册 Waker 等待空间
                    buffer_guard.register_writer_waker(waker_to_register);
                }
            }
            // 如果读端关闭，对于写端来说是 POLLERR (因为写入会 EPIPE)
            // 也可能是 POLLHUP
            if buffer_guard.read_end_closed {
                revents.insert(PollEvents::POLLERR);
                // revents.insert(PollEvents::POLLHUP);
            }
        }
        revents
    }

    async fn read<'a>(&self, mut buf: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        if !self.readable { return Err(SysErrNo::EBADF); } // 非读端
        if buf.is_empty() { return Ok(0); }

        let mut total_bytes_read = 0usize;

        let len = buf.len();
        let mut buf_iter = buf.buffers.iter_mut(); // 获取可变的迭代器
        let mut current_user_segment = buf_iter.next();
        let mut current_user_segment_offset = 0;
        loop { // 循环直到读取了数据，或遇到非阻塞情况，或EOF
            let mut ring_buffer = self.buffer.lock();
            let available_to_read_now = ring_buffer.available_read();

            if available_to_read_now > 0 {
                while let Some(segment) = current_user_segment {
                    while current_user_segment_offset < segment.len() && total_bytes_read < len {
                        if let Some(byte_val) = ring_buffer.read_byte() { // read_byte 内部会 notify_writers
                            // TODO: 使用安全的 copy_to_user_single_byte 或 UserBuffer::write_byte
                            // 假设 segment 是 &mut [u8]
                            segment[current_user_segment_offset] = byte_val;
                            current_user_segment_offset += 1;
                            total_bytes_read += 1;
                        } else { // 应该不会发生，因为 available_to_read_now > 0
                            drop(ring_buffer);
                            return Ok(total_bytes_read);
                        }
                    }
                    if current_user_segment_offset == segment.len() { // 当前 segment 写满
                        current_user_segment = buf_iter.next();
                        current_user_segment_offset = 0;
                        if current_user_segment.is_none() || total_bytes_read == len {
                            //所有用户提供的buffer都写满了
                            drop(ring_buffer);
                            return Ok(total_bytes_read);
                        }
                    } else { // 用户buffer未满，但管道暂时空了
                        drop(ring_buffer);
                        return Ok(total_bytes_read);
                    }
                }
                // 所有用户缓冲区都处理完毕
                drop(ring_buffer);
                return Ok(total_bytes_read);

            } else { // available_to_read_now == 0
                if ring_buffer.write_end_closed { // 写端已关闭，意味着EOF
                    drop(ring_buffer);
                    return Ok(total_bytes_read); // 返回已读取的（可能是0）
                }
                // 缓冲区为空，且写端未关闭
                if self.is_non_block() {
                    drop(ring_buffer);
                    if total_bytes_read > 0 { return Ok(total_bytes_read); } // 如果已读到一些，返回
                    return Err(SysErrNo::EAGAIN);
                }
                // 阻塞模式：需要等待。释放锁，让权，然后重试。
                // 这部分可以通过一个内部的 Future 实现，类似于 StdinReadyFuture
                // PipeReadReadyFuture { pipe_buffer: self.buffer.clone() }.await;
                // 为了简化，我们先用 yield_now_async，但这不如 Waker 高效。
                // 更好的做法是 read 也返回 Future，或者它期望在 poll 后被调用。
                // 假设 File::read 是在 poll 返回可读后被调用的，那么这里不应该 yield。
                // 如果它是一个独立的 async read 调用，那么需要等待机制。
                // 让我们假设 read 期望能立即读到数据，如果 poll 说了可以。
                // 如果 poll 没说可以，但还是调用了 read（例如用户直接调用），那么非阻塞返回 EAGAIN。
                // 如果是阻塞模式，就需要等待。
                // 鉴于这是 async fn，它应该返回 Pending 或等待一个 Future。
                // 我们用一个简单的 Future 来等待 Waker。
                struct PipeReadReady<'p> { pipe: &'p Pipe }
                impl<'p> Future for PipeReadReady<'p> {
                    type Output = ();
                    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                        let mut guard = self.pipe.buffer.lock();
                        if guard.available_read() > 0 || guard.write_end_closed {
                            Poll::Ready(())
                        } else {
                            guard.register_reader_waker(cx.waker());
                            trace!("Pipe Read Pending");
                            Poll::Pending
                        }
                    }
                }
                drop(ring_buffer); // 释放锁以便其他任务（如写者或唤醒者）可以访问
                PipeReadReady { pipe: self }.await; // 等待被唤醒
                // 唤醒后，loop 会重新开始，再次检查缓冲区
            }
        } // end loop
    } // end async fn read

    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo> {
        if !self.writable { return Err(SysErrNo::EBADF); } // 非写端
        if buf.is_empty() { return Ok(0); }

        let mut total_bytes_written = 0usize;
        let mut buf_iter = buf.buffers.iter(); // UserBuffer 应该是 &[&[u8]]
        let mut current_user_segment_data = buf_iter.next();
        let mut current_user_segment_offset = 0;

        loop {
            let mut ring_buffer = self.buffer.lock();
            if ring_buffer.read_end_closed { // 读端已关闭
                drop(ring_buffer);
                log::warn!("Pipe write: Read end closed (EPIPE)");
                // 向已关闭读端的管道写入应产生 SIGPIPE 信号给当前进程。
                // TODO: 发送 SIGPIPE 信号
                // crate::task::signal::send_signal_to_current_process(Signal::SIGPIPE);
                return Err(SysErrNo::EPIPE);
            }

            let available_to_write_now = ring_buffer.available_write();

            if available_to_write_now > 0 {
                while let Some(segment_data) = current_user_segment_data { // segment_data 是 &[u8]
                    while current_user_segment_offset < segment_data.len() && total_bytes_written < buf.len() {
                        if ring_buffer.available_write() > 0 { // 再次检查，因为可能在循环中被填满
                            // TODO: 从用户空间安全地读取 segment_data[current_user_segment_offset]
                            //       如果 UserBuffer 封装了安全性，则可以直接访问
                            let byte_to_write = segment_data[current_user_segment_offset];
                            ring_buffer.write_byte(byte_to_write).unwrap(); // write_byte 内部会 notify_readers
                            current_user_segment_offset += 1;
                            total_bytes_written += 1;
                        } else { // 管道在中途写满了
                            drop(ring_buffer);
                            return Ok(total_bytes_written);
                        }
                    }
                    if current_user_segment_offset == segment_data.len() {
                        current_user_segment_data = buf_iter.next();
                        current_user_segment_offset = 0;
                        if current_user_segment_data.is_none() || total_bytes_written == buf.len() {
                            drop(ring_buffer);
                            return Ok(total_bytes_written);
                        }
                    } else { // 用户数据未写完，但管道满了
                        drop(ring_buffer);
                        return Ok(total_bytes_written);
                    }
                }
                // 所有用户数据都处理完毕
                drop(ring_buffer);
                return Ok(total_bytes_written);

            } else { // available_to_write_now == 0, 缓冲区已满
                if self.is_non_block() {
                    drop(ring_buffer);
                    if total_bytes_written > 0 { return Ok(total_bytes_written); }
                    return Err(SysErrNo::EAGAIN);
                }
                // 阻塞模式：等待空间
                struct PipeWriteReady<'p> { pipe: &'p Pipe }
                impl<'p> Future for PipeWriteReady<'p> {
                    type Output = ();
                    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                        let mut guard = self.pipe.buffer.lock();
                        if guard.available_write() > 0 || guard.read_end_closed {
                            Poll::Ready(())
                        } else {
                            guard.register_writer_waker(cx.waker());

                            trace!("Pipe Wroie Pending");
                            Poll::Pending
                        }
                    }
                }
                drop(ring_buffer);
                PipeWriteReady { pipe: self }.await;
                // 唤醒后，loop 会重新开始
            }
        } // end loop
    } // end async fn write


     /// whether the file is writable
     fn writable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(self.writable)
      }
       fn readable<'a>(&'a self) -> TemplateRet<bool> {
          Ok(self.readable)
        }
        fn fstat(&self) -> Kstat {
            Kstat {
                st_mode: StMode::FIFO.bits(),
                st_nlink: 1,
                ..Kstat::default()
            }
        }
}

