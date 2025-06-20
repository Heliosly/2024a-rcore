.section .text.entry
.globl _start
_start:
    la.global $sp, boot_stack_top
    bl setbootsp

.section .bss.stack
.globl boot_stack_lower_bound
boot_stack_lower_bound:
    .space 4096 * 16
.globl boot_stack_top
boot_stack_top: