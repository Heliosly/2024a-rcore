.section .text.entry
    .globl _start
_start:
    move $tp, $a0
    
    # 分配内核栈
    slli.d $t0, $a0, 8  # t0 = hart_id << 8 (4096 * 8)
    la.abs $sp, boot_stack_top
    sub.d $sp, $sp, $t0  # sp = stack top - hart_id * stack_size

    # 分页
    la.abs $t0, boot_pagetable
    li.d $t1, 0x80
    srli.d $t0, $t0, 12
    or $t0, $t0, $t1
    csrwr $t0, 0x18  # 页表基址
    
    # TLB
    invtlb 0, $zero, $zero
    
    # 入口
    bl setbootsp

.section .bss.stack
    .globl boot_stack_lower_bound
boot_stack_lower_bound:
    .space 4096 * 16 * 2  # 2 CPUs

    .globl boot_stack_top
boot_stack_top:

.section .data
    .align 12
boot_pagetable:
    # 0x0000_0000_8000_0000 -> 0x0000_0000_8000_0000
    # 0xffff_fc00_8000_0000 -> 0x0000_0000_8000_0000
    .dword 0
    .dword 0
    .dword (0x80000 << 10) | 0xcf  # 权限位：rxw
    .zero 8 * 255
    .dword (0x80000 << 10) | 0xcf
    .zero 8 * 253

.section .text.trampoline
    .align 12
    .global sigreturn_trampoline
sigreturn_trampoline:
    li.d $a7, 139
    syscall 0