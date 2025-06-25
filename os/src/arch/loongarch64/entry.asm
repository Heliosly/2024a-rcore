.section .text.entry
    .globl _start
_start:
    # LoongArch entry point
    # a0 contains hart/core id (similar to RISC-V)
    
    # Set tp register to hart id for identification
    or $tp, $a0, $zero
    
    # Allocate kernel stack for each hart
    # Stack size = 256 bytes per hart (same as RISC-V version)
    slli.d $t0, $a0, 8        # t0 = hart_id << 8 (256 bytes per stack)
    la.global $sp, boot_stack_top
    sub.d $sp, $sp, $t0       # sp = stack_top - hart_id * stack_size

    # Setup Direct Mapping Windows for LoongArch
    # Since LoongArch uses DMW instead of page tables for kernel space
    # DMW0: Map 0x8000000000000000-0x9000000000000000 -> 0x0000000000000000-0x1000000000000000
    li.d $t0, 0x9000000000000011    # DMW0: PLV0=1, MAT=1 (Coherent), Enable=1
    csrwr $t0, 0x180                # CSR_DMW0
    
    # DMW1: Map 0xa000000000000000-0xb000000000000000 -> 0x0000000000000000-0x1000000000000000
    li.d $t0, 0xa000000000000011    # DMW1: PLV0=1, MAT=1 (Coherent), Enable=1
    csrwr $t0, 0x181                # CSR_DMW1

    # Setup page table for user space
    la.global $t0, boot_pagetable
    srli.d $t0, $t0, 12             # Convert to PFN
    csrwr $t0, 0x1c                 # CSR_PGDL (Page Global Directory Low)
    
    # Enable MMU
    li.d $t0, 0x1                   # PG=1 (Enable paging)
    csrwr $t0, 0x0                  # CSR_CRMD (Current Mode)
    
    # TLB flush
    invtlb 0x0, $zero, $zero        # Invalidate all TLB entries

    bl setbootsp

.section .bss.stack
    .globl boot_stack_lower_bound
boot_stack_lower_bound:
    .space 4096 * 16 * 2            # 2 cores, 64KB stack each

    .globl boot_stack_top
boot_stack_top:

.section .data
    .align 12
boot_pagetable:
    # LoongArch page table structure (4-level paging)
    # Map virtual addresses to physical addresses
    # 0x0000000080000000 -> 0x0000000080000000 (identity mapping)
    # 0x9000000080000000 -> 0x0000000080000000 (DMW mapping)
    .quad 0
    .quad 0
    .quad (0x80000 << 12) | 0x1f    # V=1, R=1, W=1, X=1, G=1 (LoongArch flags)
    .zero 8 * 253                   # Fill rest of first-level page table
    .quad (0x80000 << 12) | 0x1f    # Mapping for high address space
    .zero 8 * 253

.section .text.trampoline
    .align 12
    .global sigreturn_trampoline
sigreturn_trampoline:
    ori $a7, $zero, 139             # __NR_sigreturn
    syscall 0                       # LoongArch system call instruction