//!Stdin & Stdout

use core::task::Waker;

use alloc::boxed::Box;
use alloc::vec::Vec;
use async_trait::async_trait;

use super::stat::StMode;
use super::{File, Kstat, PollEvents};
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task:: yield_now ;
use crate::utils::error::{ SysErrNo, TemplateRet} ;

/// stdin file for getting chars from console
pub struct Stdin;

const LF: usize = 0x0a;
const CR: usize = 0x0d;
/// stdout file for putting chars to console
pub struct Stdout;
#[async_trait]
impl File for Stdin {
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
      Ok(true) 
    }
    fn writable<'a>(&'a self) -> TemplateRet<bool> {
         Ok(false) 
    }
    async fn read<'a>( 
        & self,                
        mut user_buf: UserBuffer<'a>  
    ) -> Result<usize, SysErrNo>{
     

        let mut c: usize;
        let mut count: usize = 0;
        let mut buf = Vec::new();
        while count < user_buf.len() {
            c = console_getchar();
            match c {
                // `c > 255`是为了兼容OPENSBI，OPENSBI未获取字符时会返回-1
                0 | 256.. => {
                    yield_now().await;
                    continue;
                }
                CR => {
                    buf.push(LF as u8);
                    count += 1;
                    break;
                }
                LF => {
                    buf.push(LF as u8);
                    count += 1;
                    break;
                }
                _ => {
                    buf.push(c as u8);
                    count += 1;
                }
            }
        }
        user_buf.write(buf.as_slice());
        Ok(count)
       
    }
    async fn write<'buf>(&self, buf: UserBuffer<'buf>) -> Result<usize, SysErrNo>{
        panic!("Cannot write to stdin!");
    }
   
    fn poll(&self, events: PollEvents, waker_to_register: &Waker) -> PollEvents {
        let mut revents = PollEvents::empty();
        
        
        if events.contains(PollEvents::POLLIN) {
            if self.readable().unwrap(){ 
                revents.insert(PollEvents::POLLIN);
            
        }
    }
      

       
        return revents;
    }

    fn fstat(&self) -> Kstat {
        Kstat {
            st_mode: StMode::FCHR.bits(),
            st_nlink: 1,
            ..Kstat::default()
        }
    }
 
}
#[async_trait]
impl File for Stdout {
    fn readable<'a>(&'a self) -> TemplateRet<bool> {
        Ok(false) 
    }
    fn writable<'a>(&'a self) -> TemplateRet<bool>{
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
    fn fstat(&self) -> Kstat {
        Kstat {
            st_mode: StMode::FCHR.bits(),
            st_nlink: 1,
            ..Kstat::default()
        }
    }
}
