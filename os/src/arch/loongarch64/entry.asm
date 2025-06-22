.section .text.entry
    .globl _start
_start:
    # LoongArch entry point
    # a0 contains hart/core id (similar to RISC-V)
    
    # Set tp register to hart id for identification
    or $tp, $a0, $zero
    
    # Allocate kernel stack for each hart
    # Stack size = 4096 * 16 = 65536 bytes per hart
    slli.d $t0, $a0, 16        # t0 = hart_id << 16 (65536 bytes per stack)
    la.global $sp, boot_stack_top
    sub.d $sp, $sp, $t0        # sp = stack_top - hart_id * stack_size

    # Setup page table for LoongArch
    # Configure DMW (Direct Mapping Window) for kernel space
    # DMW0: 0x8000000000000000 - 0x9000000000000000 -> 0x0000000000000000 - 0x1000000000000000
    li.d $t0, 0x9000000000000011    # DMW0: PLV0=1, MAT=1 (Coherent), Enable=1
    csrw 0x180, $t0                 # CSR_DMW0
    
    # DMW1: 0xa000000000000000 - 0xb000000000000000 -> 0x0000000000000000 - 0x1000000000000000  
    li.d $t0, 0xa000000000000011    # DMW1: PLV0=1, MAT=1 (Coherent), Enable=1
    csrw 0x181, $t0                 # CSR_DMW1

    # Setup basic page table (PWCL, PWCH registers)
    la.global $t0, boot_pagetable
    srli.d $t0, $t0, 12             # Convert to PFN
    csrw 0x1c, $t0                  # CSR_PGDL (Page Global Directory Low)
    
    # Enable paging
    li.d $t0, 0x5                   # PG=1, DA=1 (Direct Address Translation)
    csrw 0x0, $t0                   # CSR_CRMD (Current Mode)
    
    # TLB flush
    invtlb 0x0, $zero, $zero        # Invalidate all TLB entries

    # Jump to setbootsp
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
    # Simple identity mapping for LoongArch
    # Map 0x0000000000000000 -> 0x0000000000000000 (first 1GB)
    # Map 0x9000000000000000 -> 0x0000000000000000 (kernel direct mapping)
    
    # PGD entries (simplified 3-level page table)
    .quad 0
    .quad 0
    .quad (boot_pmd << 12) | 0x3    # Valid + Write
    .zero 8 * 509                   # Fill remaining entries
    
boot_pmd:
    # PMD entries - map first 1GB with 2MB huge pages
    .rept 512
    .quad ((. - boot_pmd) / 8 * 0x200000) | 0x83  # 2MB page, Valid, Write, Huge
    .endr

    .section .text.trampoline
    .align 12
    .global sigreturn_trampoline
sigreturn_trampoline:
    ori $a7, $zero, 139         # __NR_sigreturn
    syscall 0