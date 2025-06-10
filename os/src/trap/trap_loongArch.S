.attribute arch, "loongarch64"
    .altmacro

    .equ   FP_START,      50
    .equ   TRAPFRAME_SIZE, 688

    .macro SAVE_GP n
        st.d    $\n, \n*8($sp)
    .endm

    .macro LOAD_GP n
        ld.d    $\n, \n*8($sp)
    .endm

    .macro SAVE_FP n, m
        fst.d   $f\n, \m*8($sp)
    .endm

    .macro LOAD_FP n, m
        fld.d   $f\n, \m*8($sp)
    .endm

# Push 所有通用寄存器到 TrapContext
.macro PUSH_GENERAL_REGS
    st.d $ra,  0*8($sp)
    st.d $t0,  4*8($sp)
    st.d $t1,  5*8($sp)
    st.d $t2,  6*8($sp)
    st.d $s0,  7*8($sp)
    st.d $s1,  8*8($sp)
    st.d $a0,  9*8($sp)
    st.d $a1, 10*8($sp)
    st.d $a2, 11*8($sp)
    st.d $a3, 12*8($sp)
    st.d $a4, 13*8($sp)
    st.d $a5, 14*8($sp)
    st.d $a6, 15*8($sp)
    st.d $a7, 16*8($sp)
    st.d $s2, 17*8($sp)
    st.d $s3, 18*8($sp)
    st.d $s4, 19*8($sp)
    st.d $s5, 20*8($sp)
    st.d $s6, 21*8($sp)
    st.d $s7, 22*8($sp)
    st.d $s8, 23*8($sp)
    st.d $fp, 24*8($sp)
    st.d $s10,25*8($sp)
    st.d $s11,26*8($sp)
    st.d $t3, 27*8($sp)
    st.d $t4, 28*8($sp)
    st.d $t5, 29*8($sp)
    st.d $t6, 30*8($sp)
.endm

# 从 TrapContext 恢复所有通用寄存器
.macro POP_GENERAL_REGS
    ld.d $ra,  0*8($sp)
    ld.d $t0,  4*8($sp)
    ld.d $t1,  5*8($sp)
    ld.d $t2,  6*8($sp)
    ld.d $s0,  7*8($sp)
    ld.d $s1,  8*8($sp)
    ld.d $a0,  9*8($sp)
    ld.d $a1, 10*8($sp)
    ld.d $a2, 11*8($sp)
    ld.d $a3, 12*8($sp)
    ld.d $a4, 13*8($sp)
    ld.d $a5, 14*8($sp)
    ld.d $a6, 15*8($sp)
    ld.d $a7, 16*8($sp)
    ld.d $s2, 17*8($sp)
    ld.d $s3, 18*8($sp)
    ld.d $s4, 19*8($sp)
    ld.d $s5, 20*8($sp)
    ld.d $s6, 21*8($sp)
    ld.d $s7, 22*8($sp)
    ld.d $s8, 23*8($sp)
    ld.d $fp, 24*8($sp)
    ld.d $s10,25*8($sp)
    ld.d $s11,26*8($sp)
    ld.d $t3, 27*8($sp)
    ld.d $t4, 28*8($sp)
    ld.d $t5, 29*8($sp)
    ld.d $t6, 30*8($sp)
.endm

.macro SAVE_REGS
    PUSH_GENERAL_REGS
    csrrd   $t0, 0x6  # LOONGARCH_CSR_ERA
    csrrd   $t1, 0x5  # LOONGARCH_CSR_PRMD
    csrrd   $t2, 0x40  # LOONGARCH_CSR_TLBRSAVE
    csrwr   $zero, 0x40  # 清空 TLBRSAVE

    st.d    $t0, 32*8($sp)          # tf.sepc/era
    st.d    $t1, 31*8($sp)          # tf.sstatus/prmd
    st.d    $t2, 1*8($sp)           # tf.regs.sp

    # 保存浮点寄存器
    # .set    n, 0
    # .set    m, FP_START
    # .rept   32
    #     SAVE_FP  %n, %m
    #    .set n, n+1
    #    .set m, m+1
    # .endr
    
    csrrd   $t0, 0x7
    csrrd   $t1, 0x1
    st.d    $t0, 84*8($sp)          # tf.stval/badv
    st.d    $t1, 83*8($sp)          # tf.scause/crmd

    li.d    $t0, 1
    st.d    $t0, 85*8($sp)          # tf.trap_status = 1
.endm

.macro RESTORE_REGS
    # 恢复 era 和 prmd
    ld.d    $t0, 32*8($sp)
    ld.d    $t1, 31*8($sp)
    csrwr   $t0, 0x6  # LOONGARCH_CSR_ERA
    csrwr   $t1, 0x5  # LOONGARCH_CSR_PRMD


    POP_GENERAL_REGS
    
    # 恢复浮点寄存器
    # .set    n, 0
    # .set    m, FP_START
    # .rept   32
    #     LOAD_FP  %n, %m
    #     .set n, n+1
    #     .set m, m+1
    # .endr
    
    ld.d    $sp, 1*8($sp)
.endm

.section .text
.globl  trap_vector_base
.globl  trap_return1
.globl  user_return1
.align 2

trap_vector_base:
    # 交换 sp 和 TLBRSAVE
    csrxchg $sp, $sp, 0x40
    bnez    $sp, user_trap
    # TLBRSAVE为0，转到内核态
    b       kernel_trap

kernel_trap:
    # 内核态陷入
    csrrd   $sp, 0x40
    addi.d  $sp, $sp, -TRAPFRAME_SIZE
    SAVE_REGS
    move    $a0, $sp                 # TrapFrame指针
    li.d    $a1, 1                   # 通过trap进入
    li.d    $a2, 0                   # 内核态
    bl      trampoline
    RESTORE_REGS
    ertn

user_trap:
    SAVE_REGS
    
    li.d    $a0, 1
    st.d    $a0, 85*8($sp)           # 标记trap_status为1
    move    $a0, $sp                 # TrapFrame指针
    li.d    $a1, 1                   # 通过trap进入
    li.d    $a2, 1                   # 用户态进入
    ld.d    $sp, 33*8($sp)           # 恢复内核栈指针
    li.d    $t0, 0                   # 清零TLBRSAVE
    csrwr   $t0, 0x40
    bl      trampoline

# 内核返回
trap_return1:
    move    $sp, $a0
    
    # 恢复era和prmd
    ld.d    $t0, 32*8($sp)
    ld.d    $t1, 31*8($sp)
    csrwr   $t0, 0x6                 # LOONGARCH_CSR_ERA
    csrwr   $t1, 0x5                 # LOONGARCH_CSR_PRMD
    
    # 恢复通用寄存器
    POP_GENERAL_REGS
    
    # 恢复内核栈
    ld.d    $sp, 33*8($sp)
    
    ertn

user_return1:
    move    $sp, $a0
    
    csrwr   $a0, 0x40
    
    ld.d    $t0, 32*8($sp)
    ld.d    $t1, 31*8($sp)
    csrwr   $t0, 0x6                 # LOONGARCH_CSR_ERA
    csrwr   $t1, 0x5                 # LOONGARCH_CSR_PRMD
    
    POP_GENERAL_REGS
    
    ld.d    $sp, 1*8($sp)
    
    ertn