.section .text.entry
    .globl _start
_start:
    # rust sbi put hart id on a0
    # alloc kernel stack for each hart
    # set sp(each hart has one kstack)
    mv tp,a0
    slli t0, a0, 8  # t0 = hart_id << 16(4096 * 8)
    la sp, boot_stack_top
    sub sp, sp, t0  # sp = stack top - hart_id * stack_size

    # la sp, boot_stack_top
    # since the base addr is 0xffff_ffc0_8020_0000
    # we need to activate pagetable here in case of absolute addressing
    # satp: 8 << 60 | boot_pagetable
    la t0, boot_pagetable
    li t1, 8 << 60
    srli t0, t0, 12
    or t0, t0, t1
    csrw satp, t0
    sfence.vma

    call setbootsp

    .section .bss.stack
    .globl boot_stack_lower_bound
boot_stack_lower_bound:
    .space 4096 * 16 *2 # 2 CPUS 

    .globl boot_stack_top
boot_stack_top:

.section .data
    .align 12
boot_pagetable:
    # we need 2 pte here
    # 0x0000_0000_8000_0000 -> 0x0000_0000_8000_0000
    # 0xffff_fc00_8000_0000 -> 0x0000_0000_8000_0000
    .quad 0
    .quad 0
    .quad (0x80000 << 10) | 0xcf # VRWXAD
    .zero 8 * 255
    .quad (0x80000 << 10) | 0xcf # VRWXAD
    .zero 8 * 253

    .section .text.trampoline
    .align 12
    .global sigreturn_trampoline
sigreturn_trampoline:
    li	a7,139
    ecall