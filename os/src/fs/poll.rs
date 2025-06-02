use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::mem;

// 假设的依赖
use crate::task::{current_process, ProcessControlBlock};
use crate::fs::{FileDescriptor, PollEvents, PollFd}; // PollFdUser 是用户空间版本
use crate::mm::page_table::{copy_from_user_array, copy_to_user_bytes_exact}; // 假设有 copy_to_user_bytes_exact
use crate::mm::{VirtAddr, TranslateRefError};
use crate::utils::error::{SysErrNo, SyscallRet};
use crate::timer::{TimeVal, current_time};
use crate::task::sleeplist::{sleep_until, SleepFuture};


/// 内核中表示一个 poll 请求的结构
pub struct PollRequest {
   pub fd_index: usize,                     // 在用户传入数组中的原始索引
   pub original_user_fd: i32,             // 用户传入的原始 fd 值
   pub file_descriptor: Option< FileDescriptor>, 
   pub requested_events: PollEvents,          // 用户请求监视的事件
}
impl PollRequest {
    pub fn new(
        fd_index: usize,
        original_user_fd: i32,
        file_descriptor: Option<FileDescriptor>,
        requested_events: PollEvents,
    ) -> Self {
        Self {
            fd_index,
            original_user_fd,
            file_descriptor,
            requested_events,
        }
    }
}



#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PollFuture {
    pcb_token: usize,
    requests: Vec<PollRequest>,
    user_fds_array_ptr: *mut PollFd,
    user_fds_count: usize,
    timeout_sleep_instance: Option<SleepFuture>, // SleepFuture 的 deadline 已经是 Option<TimeVal>
}

impl PollFuture {
    pub fn new(
        pcb_token: usize,
        parsed_requests: Vec<PollRequest>,
        user_fds_array_ptr: *mut PollFd,
        user_fds_count: usize,
        timeout_deadline_opt: Option<TimeVal>, // 直接接收 Option<TimeVal>
    ) -> Self {
     
        // 如果 timeout_deadline_opt 是 None (无限等待)，我们传 None 给 sleep_until
        // 如果是 Some(deadline)，我们传 Some(deadline) 给 sleep_until
        // 所以，timeout_sleep_instance 应该在 timeout_ms != 0 时才创建
        let final_sleep_instance = if timeout_deadline_opt.is_some() || timeout_deadline_opt.is_none() /* for infinite wait */ {
            Some(sleep_until(timeout_deadline_opt)) // sleep_until(None) for infinite
        } else {
            None // Should not happen if logic is correct for timeout_ms=0 in sys_poll
        };


        Self {
            pcb_token,
            requests: parsed_requests,
            user_fds_array_ptr,
            user_fds_count,
            timeout_sleep_instance: final_sleep_instance, // 使用调整后的 sleep_instance
        }
    }

    unsafe fn write_revents_to_user(&self, results: &[(usize, PollEvents)]) -> Result<(), SysErrNo> {
        for (fd_idx, revents_val) in results.iter() {
            if *fd_idx < self.user_fds_count {
                let user_pollfd_ptr = self.user_fds_array_ptr.add(*fd_idx);
                let revents_field_ptr_in_user = &mut (*user_pollfd_ptr).revents as *mut PollEvents;
                match copy_to_user_bytes_exact(
                    self.pcb_token,
                    VirtAddr::from(revents_field_ptr_in_user as usize),
                    core::slice::from_raw_parts(
                        revents_val as *const PollEvents as *const u8,
                        core::mem::size_of::<PollEvents>()
                    ),
                ) { Ok(_) => {}, Err(_) => return Err(SysErrNo::EFAULT), }
            }
        }
        Ok(())
    }
}


impl Future for PollFuture {
    type Output = SyscallRet;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut(); // &mut PollFuture
        let mut ready_fd_count = 0;
        let mut current_results: Vec<(usize, PollEvents)> = Vec::with_capacity(this.requests.len());

        // 1. 检查所有 FD 
        for request in this.requests.iter() {
            let mut calculated_revents = PollEvents::empty();
            if request.original_user_fd < 0 {
                calculated_revents = PollEvents::empty();
            } else if let Some(fd_val) = &request.file_descriptor {
                calculated_revents = fd_val.poll(request.requested_events, cx.waker());
            } else {
                calculated_revents.insert(PollEvents::POLLNVAL);
            }
            if !calculated_revents.is_empty() {
                ready_fd_count += 1;
            }
            current_results.push((request.fd_index, calculated_revents));
        }

        // 2. 如果有任何 FD 已经就绪
        if ready_fd_count > 0 {
            match unsafe { this.write_revents_to_user(&current_results) } {
                Ok(()) => return Poll::Ready(Ok(ready_fd_count )),
                Err(e) => return Poll::Ready(Err(e)),
            }
        }

        // 3. 如果没有 FD 立即就绪，检查超时
        if let Some(sleep_future_instance) = &mut this.timeout_sleep_instance {
            // sleep_future_instance 是 &mut SleepFuture
            // Pin 住它以调用 poll。如果 SleepFuture 是 Unpin，Pin::new 是安全的。
            let mut pinned_sleep = Pin::new(sleep_future_instance); // pinned_sleep is Pin<&mut SleepFuture>

            match pinned_sleep.as_mut().poll(cx) { // pinned_sleep.as_mut() 返回 Pin<&mut SleepFuture>
                Poll::Ready(()) => { // 超时或被唤醒
                    // 通过 pinned_sleep (Pin<&mut SleepFuture>) 访问 deadline
                    // Pin<&mut T> (where T: Unpin) Derefs to T, so we can access fields directly.
                    let is_actual_timeout = pinned_sleep.deadline.is_some() && // <--- 使用 pinned_sleep
                                            current_time() >= pinned_sleep.deadline.unwrap(); // <--- 使用 pinned_sleep

                    if is_actual_timeout {
                        match unsafe { this.write_revents_to_user(&current_results) } {
                            Ok(()) => return Poll::Ready(Ok(0)),
                            Err(e) => return Poll::Ready(Err(e)),
                        }
                    } else {
                        // 不是实际时间超时
                        return Poll::Pending;
                    }
                }
                Poll::Pending => {
                    //让权
                    return Poll::Pending;
                }
            }
        } else { // 无超时设置
            if this.requests.is_empty() && this.user_fds_count > 0 {
                 match unsafe { this.write_revents_to_user(&current_results) } {
                     Ok(()) => return Poll::Ready(Ok(0)),
                     Err(e) => return Poll::Ready(Err(e)),
                 }
            }
            return Poll::Pending;
        }
    }
}