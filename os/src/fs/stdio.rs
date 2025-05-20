//!Stdin & Stdout

use alloc::boxed::Box;
use async_trait::async_trait;

use super::File;
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task:: yield_now ;
use crate::utils::error::{ASyncRet, ASyscallRet, SysErrNo, TemplateRet} ;

/// stdin file for getting chars from console
pub struct Stdin;

/// stdout file for putting chars to console
pub struct Stdout;
#[async_trait]
impl File for Stdin {
    async fn readable<'a>(&'a self) -> TemplateRet<bool> {
      Ok(true) 
    }
    async fn writable<'a>(&'a self) -> TemplateRet<bool> {
         Ok(false) 
    }
    async fn read<'a>( 
        & self,                
        mut buf: UserBuffer<'a>  
    ) -> Result<usize, SysErrNo>{
     
            assert_eq!(buf.len(), 1);
    
            loop {
                let c = console_getchar();
                if c == 0 {
                    yield_now().await;  // 使用异步 yield
                    continue;
                } else {
                    let ch = c as u8;
        unsafe {
            buf.buffers[0].as_mut_ptr().write_volatile(ch);
        }
        
                    break Ok(1);  // 返回一个成功的结果，结果是 Ok(1)
                }
            }
       
    }
    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo>{
        panic!("Cannot write to stdin!");
    }
    
}
#[async_trait]
impl File for Stdout {
    async fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(false) 
    }
    async fn writable<'a>(&'a self) -> TemplateRet<bool>{
        Ok(true) 
    }
    async fn read<'a>( 
        & self,                
        mut buf: UserBuffer<'a>  
    ) -> Result<usize, SysErrNo> {
        panic!("Cannot read from stdout!") 
    }
    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo> {
       
            for buffer in buf.buffers.iter() {
                
        // trace!("write ?t{}",core::str::from_utf8(*buffer).unwrap());
                print!("{}", core::str::from_utf8(*buffer).unwrap());
            }
            Ok(buf.len())
     
    }
}
