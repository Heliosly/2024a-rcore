.macro SAVE_REGS
    st.d    $ra, $sp, 1*8
    st.d    $tp, $sp, 2*8
    st.d    $a0, $sp, 4*8
    st.d    $a1, $sp, 5*8
    st.d    $a2, $sp, 6*8
    st.d    $a3, $sp, 7*8
    st.d    $a4, $sp, 8*8
    st.d    $a5, $sp, 9*8
    st.d    $a6, $sp, 10*8
    st.d    $a7, $sp, 11*8
    st.d    $t0, $sp, 12*8
    st.d    $t1, $sp, 13*8
    st.d    $t2, $sp, 14*8
    st.d    $t3, $sp, 15*8
    st.d    $t4, $sp, 16*8
    st.d    $t5, $sp, 17*8
    st.d    $t6, $sp, 18*8
    st.d    $t7, $sp, 19*8
    st.d    $t8, $sp, 20*8
    st.d    $fp, $sp, 22*8
    st.d    $s0, $sp, 23*8
    st.d    $s1, $sp, 24*8
    st.d    $s2, $sp, 25*8
    st.d    $s3, $sp, 26*8
    st.d    $s4, $sp, 27*8
    st.d    $s5, $sp, 28*8
    st.d    $s6, $sp, 29*8
    st.d    $s7, $sp, 30*8
    st.d    $s8, $sp, 31*8
.endm

.macro RESTORE_REGS
    ld.d    $ra, $sp, 1*8
    ld.d    $tp, $sp, 2*8
    ld.d    $a0, $sp, 4*8
    ld.d    $a1, $sp, 5*8
    ld.d    $a2, $sp, 6*8
    ld.d    $a3, $sp, 7*8
    ld.d    $a4, $sp, 8*8
    ld.d    $a5, $sp, 9*8
    ld.d    $a6, $sp, 10*8
    ld.d    $a7, $sp, 11*8
    ld.d    $t0, $sp, 12*8
    ld.d    $t1, $sp, 13*8
    ld.d    $t2, $sp, 14*8
    ld.d    $t3, $sp, 15*8
    ld.d    $t4, $sp, 16*8
    ld.d    $t5, $sp, 17*8
    ld.d    $t6, $sp, 18*8
    ld.d    $t7, $sp, 19*8
    ld.d    $t8, $sp, 20*8
    ld.d    $fp, $sp, 22*8
    ld.d    $s0, $sp, 23*8
    ld.d    $s1, $sp, 24*8
    ld.d    $s2, $sp, 25*8
    ld.d    $s3, $sp, 26*8
    ld.d    $s4, $sp, 27*8
    ld.d    $s5, $sp, 28*8
    ld.d    $s6, $sp, 29*8
    ld.d    $s7, $sp, 30*8
    ld.d    $s8, $sp, 31*8
.endm

.section .text
.globl loongarch_trap_vector
.globl loongarch_trap_return
.globl loongarch_user_return

.align 2
loongarch_trap_vector:
    # 交换 sp 和内核栈指针
    csrxchg $sp, $sp, 0x30
    bnez    $sp, user_trap
    j       kernel_trap

kernel_trap:
    csrrd   $sp, 0x30
    addi    $sp, $sp, -32*8
    SAVE_REGS

    move    $a0, $sp
    bl      trampoline
    RESTORE_REGS
    addi    $sp, $sp, 32*8
    ertn

user_trap:
    SAVE_REGS
    
    csrrd   $t0, 0x6        # ERA
    csrrd   $t1, 0x1        # PRMD  
    csrrd   $t2, 0x5        # ESTAT
    csrrd   $t3, 0x7        # BADV
    
    st.d    $t0, $sp, 32*8  # era
    st.d    $t1, $sp, 33*8  # prmd
    st.d    $t2, $sp, 35*8  # estat
    st.d    $t3, $sp, 36*8  # badv
    
    move    $a0, $sp
    li      $a1, 1          # has_trap = true
    li      $a2, 1          # from_user = true
    bl      trampoline

loongarch_trap_return:
    move    $sp, $a0
    
    ld.d    $t0, $sp, 32*8  # era
    ld.d    $t1, $sp, 33*8  # prmd
    csrwr   $t0, 0x6        # ERA
    csrwr   $t1, 0x1        # PRMD
    
    RESTORE_REGS
    ld.d    $sp, $sp, 3*8   # 恢复用户栈指针
    ertn

loongarch_user_return:
    move    $sp, $a0
    
    ld.d    $t0, $sp, 32*8
    ld.d    $t1, $sp, 33*8
    csrwr   $t0, 0x6
    csrwr   $t1, 0x1
    
    RESTORE_REGS
    ld.d    $sp, $sp, 3*8
    ertn