use core::sync::atomic::AtomicUsize;

use alloc::{sync::Arc, vec};
use alloc::vec:: Vec;

use crate::fs::{Stdin, Stdout};
use crate::{config::MAX_FD_NUM, fs::{FileClass, FileDescriptor, OpenFlags}, utils::error::SysErrNo};


pub struct FdManage{
    pub table: Vec<Option<FileDescriptor>>,
    soft_limit: AtomicUsize,
    hard_limit: AtomicUsize,
}

impl FdManage {
    pub fn new(soft_limit:usize,hard_limit:usize,table:Vec<Option<FileDescriptor>>) -> Self {
        FdManage{
          soft_limit: AtomicUsize::new(soft_limit),
          table:  table,
            hard_limit:AtomicUsize::new(hard_limit),
          
        }
    }
    pub fn get_hard_limit(&self) -> usize {
        self.hard_limit.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn get_soft_limit(&self) -> usize {
        self.soft_limit.load(core::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_limit(&self, soft_limit: usize, hard_limit: usize) {
        self.soft_limit.store(soft_limit, core::sync::atomic::Ordering::Relaxed);
        self.hard_limit.store(hard_limit, core::sync::atomic::Ordering::Relaxed);
    }
    pub fn new_with_stdio() -> Self {
        FdManage::new(
            128,
            256,
            
                vec![
                    Some(FileDescriptor {
                        flags: OpenFlags::O_RDONLY,
                        file: FileClass::Abs(Arc::new(Stdin)),
                    }),
                    // stdout: 只写
                    Some(FileDescriptor {
                        flags: OpenFlags::O_WRONLY,
                        file: FileClass::Abs(Arc::new(Stdout)),
                    }),
                    // stderr: 只写
                    Some(FileDescriptor {
                        flags: OpenFlags::O_WRONLY,
                        file: FileClass::Abs(Arc::new(Stdout)),
                    }),
                
            ],
        )
    }
  
    
    pub fn from_another(another: &FdManage) -> Self {
        Self {
            table: another.table.clone(),
            soft_limit: AtomicUsize::new(another.soft_limit.load(core::sync::atomic::Ordering::Relaxed)),
            hard_limit: AtomicUsize::new(another.hard_limit.load(core::sync::atomic::Ordering::Relaxed)),
        }
    }
    /// 获取文件描述符表（fd_table）的长度。
    pub fn len(&self) -> usize {
        self.table.len()
    }
    /// 通过path
    /// 检查 fd_table 中是否存在指定的文件描述符。
    /// 持有 fd_table 的锁。
    

    /// 获取 fd_table[fd] 对应的文件句柄（Arc 克隆）。
    ///
    /// 不会修改表里原有的 Option。
    ///
    /// # Arguments
    ///
    /// * `fd`: 文件描述符。
    ///
    /// # Returns
    ///
    /// * `Some(FileDescriptor)`: 如果存在，则返回文件句柄的克隆。
    /// * `None`: 如果 fd_table 中不存在该 fd，或该 fd 对应的项为 None。
    pub fn get_file(&self, fd: usize) -> Result<FileDescriptor, SysErrNo> {
        match self.table.get(fd) {
            Some(Some(file_desc)) => Ok(file_desc.clone()),
            _ => Err(SysErrNo::EBADF),
        }
    }

    /// 分配一个大于或等于 min_fd 的最小可用文件描述符。
    ///
    /// # Arguments
    /// * `min_fd`: 新文件描述符的最小编号。
    ///
    /// # Returns
    /// * `Some(usize)`: 如果找到可用的 fd，则返回其编号。
    /// * `None`: 如果从 min_fd 到 PROCESS_MAX_FDS-1 都没有可用的 fd。
    ///
    /// 注意：此函数仅仅 *找到* 一个可用的 fd 编号。
    /// 调用者负责实际获取 fd_table 的锁，并在必要时调整其大小，
    /// 然后将 FileDescriptor 实例插入到返回的 fd 编号对应的位置。
    pub   fn alloc_fd_from(&self, min_fd: usize) -> Option<usize> {
      

        // 从 min_fd 开始向上搜索，直到 PROCESS_MAX_FDS 上
        for fd_candidate in min_fd..MAX_FD_NUM {
            if fd_candidate < self.table.len() {
                // 如果 fd_candidate 在当前 fd_table 的范围内
                if self.table[fd_candidate].is_none() {
                    // 找到了一个空的槽位
                    return Some(fd_candidate);
                }
            } else {
                // 如果 fd_candidate 超出了当前 fd_table 的范围，
                // 但仍在 PROCESS_MAX_FDS 之内，那么这个槽位是可用的。
                // fd_table 将在稍后实际插入文件时被扩展。
                return Some(fd_candidate);
            }
        }

        // 如果循环完成都没有找到，说明从 min_fd 开始的所有槽位都被占用了
        // (或者 min_fd >= PROCESS_MAX_FDS)
        None
    }
    /// 向文件描述符表（fd_table）中添加一个文件描述符。
    ///
    /// 如果 `pos` 小于当前 `fd_table` 的长度，则替换该位置的文件句柄。
    /// 否则，扩展 `fd_table` 到 `pos + 1` 的长度，并在 `pos` 位置插入文件句柄。
    ///
    /// # Arguments
    ///
    /// * `fd`: 要添加的文件句柄。
    /// * `pos`: 要添加到的文件描述符位置。
    ///
    /// # Returns
    ///
    /// * `Some(FileDescriptor)`: 如果 `pos` 位置原来存在文件句柄，则返回原来的文件句柄。
    /// * `None`: 如果 `pos` 位置原来没有文件句柄。
    ///
    /// # Panics
    ///
    /// 如果 `pos` 大于 `MAX_FD_NUM`，则会发生 panic。
    ///
    /// 注意：此函数内部持有 `fd_table` 的锁。
    pub   fn add_fd(&mut self, fd: FileDescriptor,pos:usize) ->Option<FileDescriptor>  {
      
        if pos>MAX_FD_NUM{
            panic!("fd out of range");
        }
        if pos<self.table.len(){
           self.table[pos].replace(fd)
         }
         else{
            self.table.resize(pos+1, None);
            self.table[pos].replace(fd)
         }
    }
    /// “取出” fd_table[fd] 对应的文件句柄，
    /// 表中该项会被置为 None，相当于关闭/移除它。
    ///
    /// # Arguments
    ///
    /// * `fd`: 要取出的文件描述符。
    ///
    /// # Returns
    ///
    /// * `Some(FileDescriptor)`: 如果存在，则返回文件句柄。
    /// * `None`: 如果 fd_table 中不存在该 fd，或该 fd 对应的项为 None。
    pub  fn take_file(&mut self, fd: usize) -> Option<FileDescriptor> {
        self.table.get_mut(fd)
             .and_then(|opt| opt.take())
    }


    /// 分配一个文件描述符。
    pub fn alloc_fd(&mut self) -> usize {
        let  fd_table = &mut self.table;
        if let Some(fd) = (0..fd_table.len()).find(|fd| fd_table[*fd].is_none()) {
            fd
        } else {
            fd_table.push(None);
            fd_table.len() - 1
        }
    }
 
    

/// 在文件描述符表中查找与指定路径匹配的文件描述符。
///
/// # 参数
/// * `path`: 要查找的文件路径。
///
/// # 返回值
/// 如果找到与路径匹配的文件描述符，则返回其索引（`Some(usize)`）；
/// 如果未找到，则返回 `None`。
///
/// # 描述
/// 此函数会遍历文件描述符表（`fd_table`），检查每个文件描述符是否与指定路径匹配。
/// 如果文件描述符存在且其关联的文件路径与 `path` 相同，则返回该文件描述符的索引。
///
/// # 注意
/// - 如果文件描述符为空或文件路径获取失败，则跳过该项。
/// - 此函数不会修改文件描述符表。
    pub fn find_fd(&self,path: &str) -> Option<usize> {
        self.table.iter().position(|fd| {
            if let Some(file_desc) = fd {
                match file_desc.file(){
                    Ok(f) => f.get_path()==path,
                    Err(_) => false,
                }
            } else {
                false
            }
        })
    }   

   
}