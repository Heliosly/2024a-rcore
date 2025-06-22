use core::future::poll_fn; // 引入 poll_fn
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;
use async_trait::async_trait;

use crate::fs::{File, Kstat, OpenFlags, PollEvents};
use crate::fs::stat::StMode;
use crate::mm::UserBuffer;
use crate::sync::Mutex;
use crate::task::yield_now;
use crate::utils::error::{SysErrNo, TemplateRet};

const RING_CAPACITY: usize = 0x4000;

// RingBuffer 结构体保持不变
struct RingBuffer {
    buf: [u8; RING_CAPACITY],
    head: usize,
    tail: usize,
    len: usize,
    read_closed: bool,
    write_closed: bool,
    readers: Vec<Waker>,
    writers: Vec<Waker>,
}

// RingBuffer 的 impl 块保持不变...
impl RingBuffer {
    fn new() -> Self {
        Self {
            buf: [0; RING_CAPACITY], head: 0, tail: 0, len: 0,
            read_closed: false, write_closed: false,
            readers: Vec::new(), writers: Vec::new(),
        }
    }

    fn available_read(&self) -> usize { self.len }
    fn available_write(&self) -> usize { RING_CAPACITY - self.len }
    
    fn write_bytes(&mut self, data: &[u8]) -> usize {
        let was_empty = self.len == 0;
        let write_len = data.len().min(self.available_write());
        if write_len == 0 { return 0; }
        let (p1, p2) = self.buf.split_at_mut(self.tail);
        let space_in_p2 = p2.len();
        if write_len <= space_in_p2 {
            p2[..write_len].copy_from_slice(&data[..write_len]);
        } else {
            p2.copy_from_slice(&data[..space_in_p2]);
            p1[..write_len - space_in_p2].copy_from_slice(&data[space_in_p2..write_len]);
        }
        self.tail = (self.tail + write_len) % RING_CAPACITY;
        self.len += write_len;
        if was_empty {
            self.notify_readers();
        }
        write_len
    }

    fn read_bytes(&mut self, target: &mut [u8]) -> usize {
        let was_full = self.len == RING_CAPACITY;
        let read_len = target.len().min(self.available_read());
        if read_len == 0 { return 0; }
        
        let (p1, p2) = self.buf.split_at(self.head);
        let data_in_p2 = p2.len();
        if read_len <= data_in_p2 {
            target[..read_len].copy_from_slice(&p2[..read_len]);
        } else {
            target[..data_in_p2].copy_from_slice(p2);
            target[data_in_p2..read_len].copy_from_slice(&p1[..read_len-data_in_p2]);
        }
        self.head = (self.head + read_len) % RING_CAPACITY;
        self.len -= read_len;

        if was_full {
            self.notify_writers();
        }
        read_len
    }

    fn close_read(&mut self) {
        if !self.read_closed {
            self.read_closed = true;
            self.notify_writers();
        }
    }
    fn close_write(&mut self) {
        if !self.write_closed {
            self.write_closed = true;
            self.notify_readers();
        }
    }

    fn notify_readers(&mut self) {
        for w in self.readers.drain(..) { w.wake(); }
    }
    fn notify_writers(&mut self) {
        for w in self.writers.drain(..) { w.wake(); }
    }
    
    fn register_reader(&mut self, w: &Waker) {
        // println!("register reder");
        if !self.readers.iter().any(|rw| rw.will_wake(w)) {
            self.readers.push(w.clone());
        }
    }

    fn register_writer(&mut self, w: &Waker) {

        // println!("register writer");
        if !self.writers.iter().any(|ww| ww.will_wake(w)) {
            self.writers.push(w.clone());
        }
    }
}


pub struct PipeReader {
    inner: Arc<Mutex<RingBuffer>>,
    flags: OpenFlags,
}
pub struct PipeWriter {
    inner: Arc<Mutex<RingBuffer>>,
    flags: OpenFlags,
}
pub fn make_pipe(flags: OpenFlags) -> (Arc<PipeReader>, Arc<PipeWriter>) {
    let buf = Arc::new(Mutex::new(RingBuffer::new()));
    let reader = Arc::new(PipeReader {
        inner: Arc::clone(&buf),
        flags,
    });
    let writer = Arc::new(PipeWriter {
        inner: buf,
        flags,
    });
    (reader, writer)
}


#[async_trait]
impl File for PipeReader {
    fn readable(&self) -> TemplateRet<bool> { Ok(true) }
    fn writable(&self) -> TemplateRet<bool> { Ok(false) }

    fn poll(&self, events: PollEvents, w: &Waker) -> PollEvents {
        let mut re = PollEvents::empty();
        let mut guard = self.inner.lock();
        if events.contains(PollEvents::POLLIN) {
            if guard.available_read() > 0 {
                re.insert(PollEvents::POLLIN);
            } else if guard.write_closed {
                re.insert(PollEvents::POLLIN | PollEvents::POLLHUP);
            } else  {
                guard.register_reader(w);
            }
        }
        re
    }

    async fn read<'a>(&self, mut ub: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        let nonblk = self.flags.contains(OpenFlags::O_NONBLOCK);
        
        poll_fn(|cx: &mut Context<'_>| {
            let mut guard = self.inner.lock();
            
            let avail = guard.available_read();
            if avail > 0 {
                let read_len = avail.min(ub.len());
                let mut temp_buf = vec![0u8; read_len];
                let actual_read = guard.read_bytes(&mut temp_buf);
                
                // --- 修正点 1 (改进) ---
                // 使用 write_all 以便利用其错误处理能力
                ub.write_all(&temp_buf[..actual_read])?;
                
                return Poll::Ready(Ok(actual_read));
            }

            if guard.write_closed {
                return Poll::Ready(Ok(0));
            }
            if nonblk {
                return Poll::Ready(Err(SysErrNo::EAGAIN));
            }

            guard.register_reader(cx.waker());
            Poll::Pending
        }).await
    }

    async fn write<'a>(&self, _: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        Err(SysErrNo::EBADF)
    }

    fn fstat(&self) -> Kstat {
        Kstat { st_mode: StMode::FIFO.bits(), st_nlink: 1, ..Default::default() }
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.inner.lock().close_read();
    }
}

#[async_trait]
impl File for PipeWriter {
    fn readable(&self) -> TemplateRet<bool> { Ok(false) }
    fn writable(&self) -> TemplateRet<bool> { Ok(true) }

    fn poll(&self, events: PollEvents, w: &Waker) -> PollEvents {
        // ... poll 方法保持不变
        let mut re = PollEvents::empty();
        let mut guard = self.inner.lock();
        if events.contains(PollEvents::POLLOUT) {
            if guard.available_write() > 0 {
                re.insert(PollEvents::POLLOUT);
            } else if guard.read_closed {
                re.insert(PollEvents::POLLOUT | PollEvents::POLLERR);
            } else if !self.flags.contains(OpenFlags::O_NONBLOCK) {
                guard.register_writer(w);
            }
        }
        re
    }
    
    async fn read<'a>(&self, _: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        Err(SysErrNo::EBADF)
    }

    async fn write<'a>(&self, ub: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        let nonblk = self.flags.contains(OpenFlags::O_NONBLOCK);
        let mut written_total = 0;
        
        loop {
            // 注意 poll_fn 闭包的返回值是 Poll<Result<usize, SysErrNo>>
            let result: Result<usize, SysErrNo> = poll_fn(|cx: &mut Context<'_>| {
                let mut guard = self.inner.lock();
                
                if guard.read_closed {
                    return Poll::Ready(Err(SysErrNo::EPIPE));
                }
                
                let avail_to_write = guard.available_write();
                if avail_to_write > 0 {
                    let remaining_in_ub = ub.len() - written_total;
                    let write_len = remaining_in_ub.min(avail_to_write);
                    
                    if write_len == 0 {
                        return Poll::Ready(Ok(0));
                    }

                    // --- 修正点 2 (核心错误修复) ---
                    // 使用我们刚刚实现的 read_from 方法，它完全符合这里的需求
                    let data_chunk = ub.read_from(written_total, write_len);
                    let actual_written = guard.write_bytes(&data_chunk);
                    return Poll::Ready(Ok(actual_written));
                }

                if nonblk {
                    // 如果已经写入了一些数据，应该返回成功，而不是EAGAIN
                    if written_total > 0 {
                        return Poll::Ready(Ok(0)); // 用Ok(0)来终止循环，外面会返回 written_total
                    } else {
                        return Poll::Ready(Err(SysErrNo::EAGAIN));
                    }
                }

                guard.register_writer(cx.waker());
                Poll::Pending
            }).await;

            let just_written = result?;
            if just_written == 0 {
                break; // 表示所有数据都写入，或者非阻塞模式下本轮无法再写
            }
            written_total += just_written;

            if nonblk {
                break;
            }
            yield_now().await;
        }
        
        Ok(written_total)
    }

    fn fstat(&self) -> Kstat {
        Kstat { st_mode: StMode::FIFO.bits(), st_nlink: 1, ..Default::default() }
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.inner.lock().close_write();
    }
}




/// 代表 socketpair 的一个端点
/// 它内部持有一个用于写的管道和一个用于读的管道
pub struct Socket {
    reader: Arc<PipeReader>,
    writer: Arc<PipeWriter>,
}

/// 创建一对相互连接的 socket
pub fn make_socketpair(flags: OpenFlags) -> (Arc<Socket>, Arc<Socket>) {
    // 创建第一个单向管道，用于 A -> B 的通信
    let (reader_a_to_b, writer_a_to_b) = make_pipe(flags);
    // 创建第二个单向管道，用于 B -> A 的通信
    let (reader_b_to_a, writer_b_to_a) = make_pipe(flags);

    // 将末端捆绑成两个全双工的 Socket
    let socket_a = Arc::new(Socket {
        reader: reader_b_to_a, // A 从 B->A 管道读取
        writer: writer_a_to_b, // A 向 A->B 管道写入
    });

    let socket_b = Arc::new(Socket {
        reader: reader_a_to_b, // B 从 A->B 管道读取
        writer: writer_b_to_a, // B 向 B->A 管道写入
    });

    (socket_a, socket_b)
}

#[async_trait]
impl File for Socket {
    /// socket 是可读的 (因为它内部有一个 PipeReader)
    fn readable(&self) -> TemplateRet<bool> {
        Ok(true)
    }

    /// socket 是可写的 (因为它内部有一个 PipeWriter)
    fn writable(&self) -> TemplateRet<bool> {
        Ok(true)
    }

    /// 将 poll 请求委托给内部的 reader 和 writer
    fn poll(&self, events: PollEvents, w: &Waker) -> PollEvents {
        // 分别对读和写事件进行 poll，然后合并结果
        let read_events = self.reader.poll(events, w);
        let write_events = self.writer.poll(events, w);
        
        // 合并两个方向的事件
        // println!("[socket poll]read_events: {:?}, write_events: {:?}, request events:{:?}", read_events, write_events,events);
        read_events | write_events
    }

    /// 将 read 操作委托给内部的 reader
    async fn read<'a>(&self, ub: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        self.reader.read(ub).await
    }

    /// 将 write 操作委托给内部的 writer
    async fn write<'a>(&self, ub: UserBuffer<'a>) -> Result<usize, SysErrNo> {
        self.writer.write(ub).await
    }

    fn fstat(&self) -> Kstat {
        // 返回 FSOCK 表示这是一个 socket
        Kstat { st_mode: crate::fs::stat::StMode::FSOCK.bits(), st_nlink: 1, ..Default::default() }
    }
}
