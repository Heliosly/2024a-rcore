# To create the sigreturn trampoline
.equ __NR_sigreturn, 139



.section .text.signal_trampoline, "ax"  # "ax": allocatable+executable
.globl start_signal_trampoline

.balign 4
start_signal_trampoline:
    li a7, __NR_sigreturn
    li a0, 0
    ecall