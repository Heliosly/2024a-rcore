//!Stdin & Stdout

use alloc::boxed::Box;

use super::File;
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task:: yield_now ;
use crate::utils::error::{ASyncRet, ASyscallRet} ;

/// stdin file for getting chars from console
pub struct Stdin;

/// stdout file for putting chars to console
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) ->ASyncRet<bool> {
        Box::pin(async { Ok(true) })
    }
    fn writable(&self) -> ASyncRet<bool> {
        Box::pin(async { Ok(false) })
    }
    fn read(&self, mut  user_buf: UserBuffer) -> ASyscallRet{
        Box::pin(async move {
            assert_eq!(user_buf.len(), 1);
    
            loop {
                let c = console_getchar();
                if c == 0 {
                    yield_now().await;  // 使用异步 yield
                    continue;
                } else {
                    let ch = c as u8;
        unsafe {
            user_buf.buffers[0].as_mut_ptr().write_volatile(ch);
        }
        
                    break Ok(1);  // 返回一个成功的结果，结果是 Ok(1)
                }
            }
        })
    }
    fn write(&self, _user_buf: UserBuffer) -> ASyscallRet {
        panic!("Cannot write to stdin!");
    }
    
}

impl File for Stdout {
    fn readable(&self) -> ASyncRet<bool> {
        Box::pin(async { Ok(false) })
    }
    fn writable(&self) -> ASyncRet<bool> {
        Box::pin(async { Ok(true) })
    }
    fn read(&self, _user_buf: UserBuffer) -> ASyscallRet {
        Box::pin(async { panic!("Cannot read from stdout!") })
    }
    fn write(&self, user_buf: UserBuffer) -> ASyscallRet {
        Box::pin(async move {
            for buffer in user_buf.buffers.iter() {
                
        // trace!("write ?t{}",core::str::from_utf8(*buffer).unwrap());
                print!("{}", core::str::from_utf8(*buffer).unwrap());
            }
            Ok(user_buf.len())
        })
    }
}
