use alloc::vec;

use crate::mm::{MapArea, MemorySet, PageTable, VirtPageNum};


/// 验证栈数据在父进程和子进程之间的一致性
pub fn validate_stack_consistency(
    parent_area: &MapArea,
    parent_pt: &PageTable,
    child_memory_set: &MemorySet
) {
    let stack_start = parent_area.vpn_range.get_start();
    let stack_end = parent_area.vpn_range.get_end();
    
    // 选择几个关键点进行验证
    let sample_points = vec![
        stack_start, 
        (stack_start.0 + (stack_end.0 - stack_start.0) / 2).into(),
        (stack_end.0 - 1).into(),
    ];
    
    for &vpn in &sample_points {
        // 获取父进程的栈数据
        let parent_ppn = parent_pt.translate(vpn).expect("Parent stack page not mapped").ppn();
        let parent_data = parent_ppn.get_bytes_array();
        
        // 获取子进程的栈数据
        let child_ppn = child_memory_set.translate(vpn).expect("Child stack page not mapped").ppn();
        let child_data = child_ppn.get_bytes_array();
        
        // 比较数据
        if parent_data != child_data {
            panic!("Stack data mismatch at VPN {:#x}\nParent: {:?}\nChild: {:?}",
                   vpn.0, &parent_data[..32], &child_data[..32]);
        }
        
        // 检查是否标记为 COW
        let child_pte = child_memory_set.page_table.translate(vpn).expect("Child PTE missing");
        if !child_pte.is_cow() {
            panic!("Stack page not marked as COW at VPN {:#x},pte:{:?}", vpn.0,child_pte.flags());
        }
    }
    
   
}

/// 获取当前栈指针的 VPN
fn get_current_stack_pointer_vpn() -> Option<VirtPageNum> {
    let sp: usize;
    unsafe {
        core::arch::asm!("mv {}, sp", out(reg) sp);
    }
    Some(VirtPageNum::from(sp))
}