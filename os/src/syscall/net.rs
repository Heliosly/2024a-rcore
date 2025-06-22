use alloc::{format, string::ToString};

use crate::{
    fs::{ net::make_socket, FileClass, FileDescriptor, OpenFlags},
    mm::{put_data, translated_refmut},
    task::{current_process, current_task, current_token}, utils::error::{SysErrNo, SyscallRet},
};
use log::debug;

pub async  fn sys_socket(_domain: u32, _type: u32, _protocol: u32) -> SyscallRet {
    let proc = current_process();
    let new_fd =proc.alloc_fd().await?;
    let close_on_exec = (_type & 0o2000000) == 0o2000000;
    let non_block = (_type & 0o4000) == 0o4000;
    let mut flags = OpenFlags::empty();
    if close_on_exec {
        flags |= OpenFlags::FD_CLOEXEC;
    }
    if non_block {
        flags |= OpenFlags::O_NONBLOCK;
    }
    
    proc.fd_table.lock().await.add_fd(
        FileDescriptor::new(flags, FileClass::Abs(make_socket())),

        new_fd,
    )?;
    
    Ok(new_fd)
}

pub fn sys_bind(_sockfd: usize, _addr: *const u8, _addrlen: u32) -> SyscallRet {
    Ok(0)
}

pub fn sys_getsockname(_sockfd: usize, _addr: *const u8, _addrlen: u32) -> SyscallRet {
    Ok(0)
}

pub fn sys_getpeername(_sockfd: usize, _addr: *const u8, _addrlen: u32) -> SyscallRet {
    Err(SysErrNo::Default)
}

pub fn sys_setsockopt(
    _sockfd: usize,
    _level: u32,
    _optname: u32,
    _optcal: *const u8,
    _optlen: u32,
) -> SyscallRet {
    Ok(0)
}

pub fn sys_sendto(
    _sockfd: usize,
    _buf: *const u8,
    _len: usize,
    _flags: u32,
    _dest_addr: *const u8,
    _addrlen: u32,
) -> SyscallRet {
    Ok(1)
}

pub async  fn sys_recvfrom(
    _sockfd: usize,
    buf: *mut u8,
    _len: usize,
    _flags: u32,
    _src_addr: *const u8,
    _addrlen: u32,
) -> SyscallRet {
    trace!(
        "[sys_recvfrom] sockfd: {}, buf: {:p}, len: {}, flags: {:#x}, src_addr: {:p}, addrlen: {}",
        _sockfd,
        buf,
        _len,
        _flags,
        _src_addr,
        _addrlen
    );
    let proc = current_process();
    proc.manual_alloc_type_for_lazy(buf).await?;
    let token  = proc.get_user_token().await;
    unsafe {
        put_data(token, buf, b'x')?;
        put_data(token, buf.add(1), b'0')?;
    }
    Ok(1)
}

pub fn sys_listen(_sockfd: usize, _backlog: u32) -> SyscallRet {
    trace!("[sys_listen] sockfd: {}, backlog: {}", _sockfd, _backlog);
    Ok(0)
}

pub fn sys_connect(_sockfd: usize, _addr: *const u8, _addrlen: u32) -> SyscallRet {
    trace!(
        "[sys_connect] sockfd: {}, addr: {:p}, addrlen: {}",
        _sockfd,
        _addr,
        _addrlen
    );
    Ok(0)
}

pub fn sys_accept(_sockfd: usize, _addr: *const u8, _addrlen: u32) -> SyscallRet {
    trace!(
        "[sys_accept] sockfd: {}, addr: {:p}, addrlen: {}",
        _sockfd,
        _addr,
        _addrlen
    );
    Ok(0)
}

pub fn sys_accept4(_sockfd: usize, _addr: *const u8, _addrlen: u32, _flags: u32) -> SyscallRet {
    trace!(
        "[sys_accept4] sockfd: {}, addr: {:p}, addrlen: {}, flags: {:#x}",
        _sockfd,
        _addr,
        _addrlen,
        _flags
    );
    Ok(0)
}

pub fn sys_sendmsg(_sockfd: usize, _addr: *const u8, _flags: u32) -> SyscallRet {
    trace!(
        "[sys_sendmsg] sockfd: {}, addr: {:p}, flags: {:#x}",
        _sockfd,
        _addr,
        _flags
    );
    Ok(0)
}

pub async  fn sys_socketpair(domain: u32, stype: u32, protocol: u32, sv: *mut u32) -> SyscallRet {
    info!(
        "[sys_socketpair] domain is {}, type is {}, protocol is {}, sv is {}",
        domain, stype, protocol, sv as usize
    );

    let task = current_task();
    let proc = current_process();
    let token= proc.get_user_token().await;

    let (socket1, socket2) = crate::fs::net::make_socketpair( OpenFlags::O_NONBLOCK);
    let close_on_exec = (stype & 0o2000000) == 0o2000000;
    let non_block = (stype & 0o4000) == 0o4000;
    let mut flags = OpenFlags::empty();
    if close_on_exec {
        flags |= OpenFlags::FD_CLOEXEC;
    }
    if non_block {
        flags |= OpenFlags::O_NONBLOCK;
    }
    proc.manual_alloc_type_for_lazy(sv).await?;
    let new_fd1 = proc.fd_table.lock().await.alloc_fd()?;
    proc
        .fd_table.lock()
        .await.add_fd(FileDescriptor::new(flags, FileClass::Abs(socket1)),new_fd1, )?;
        let new_fd2 = proc.fd_table.lock().await.alloc_fd()?;
    proc
        .fd_table.lock()

        .await.add_fd( FileDescriptor::new(flags, FileClass::Abs(socket2)),new_fd2)?;
 

    *translated_refmut(token, sv)? = new_fd1 as u32;
    *translated_refmut(token, unsafe { sv.add(1) })? = new_fd2 as u32;

    Ok(0)
}
