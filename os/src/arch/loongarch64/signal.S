# To create the sigreturn trampoline
.equ __NR_sigreturn, 139

.section .text.signal_trampoline, "ax"  # "ax": allocatable+executable
.globl start_signal_trampoline

.balign 4
start_signal_trampoline:
    ori $a7, $zero, __NR_sigreturn
    ori $a0, $zero, 0
    syscall 0