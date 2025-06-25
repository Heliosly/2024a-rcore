.equ TRAPFRAME_SIZE, 688

.macro SAVE_GP n
    st.d $r\n, $sp, \n*8
.endm

.macro LOAD_GP n
    ld.d $r\n, $sp, \n*8
.endm

.macro SAVE_FP n, m
    fst.d $f\n, $sp, \m*8
.endm

.macro LOAD_FP n, m
    fld.d $f\n, $sp, \m*8
.endm

.macro PUSH_GENERAL_REGS
    st.d $r1,  $sp, 1*8
    st.d $r2,  $sp, 2*8
    st.d $r3,  $sp, 3*8
    st.d $r4,  $sp, 4*8
    st.d $r5,  $sp, 5*8
    st.d $r6,  $sp, 6*8
    st.d $r7,  $sp, 7*8
    st.d $r8,  $sp, 8*8
    st.d $r9,  $sp, 9*8
    st.d $r10, $sp, 10*8
    st.d $r11, $sp, 11*8
    st.d $r12, $sp, 12*8
    st.d $r13, $sp, 13*8
    st.d $r14, $sp, 14*8
    st.d $r15, $sp, 15*8
    st.d $r16, $sp, 16*8
    st.d $r17, $sp, 17*8
    st.d $r18, $sp, 18*8
    st.d $r19, $sp, 19*8
    st.d $r20, $sp, 20*8
    st.d $r21, $sp, 21*8
    st.d $r22, $sp, 22*8
    st.d $r23, $sp, 23*8
    st.d $r24, $sp, 24*8
    st.d $r25, $sp, 25*8
    st.d $r26, $sp, 26*8
    st.d $r27, $sp, 27*8
    st.d $r28, $sp, 28*8
    st.d $r29, $sp, 29*8
    st.d $r30, $sp, 30*8
    st.d $r31, $sp, 31*8
.endm

.macro POP_GENERAL_REGS
    ld.d $r1,  $sp, 1*8
    ld.d $r2,  $sp, 2*8
    ld.d $r3,  $sp, 3*8
    ld.d $r4,  $sp, 4*8
    ld.d $r5,  $sp, 5*8
    ld.d $r6,  $sp, 6*8
    ld.d $r7,  $sp, 7*8
    ld.d $r8,  $sp, 8*8
    ld.d $r9,  $sp, 9*8
    ld.d $r10, $sp, 10*8
    ld.d $r11, $sp, 11*8
    ld.d $r12, $sp, 12*8
    ld.d $r13, $sp, 13*8
    ld.d $r14, $sp, 14*8
    ld.d $r15, $sp, 15*8
    ld.d $r16, $sp, 16*8
    ld.d $r17, $sp, 17*8
    ld.d $r18, $sp, 18*8
    ld.d $r19, $sp, 19*8
    ld.d $r20, $sp, 20*8
    ld.d $r21, $sp, 21*8
    ld.d $r22, $sp, 22*8
    ld.d $r23, $sp, 23*8
    ld.d $r24, $sp, 24*8
    ld.d $r25, $sp, 25*8
    ld.d $r26, $sp, 26*8
    ld.d $r27, $sp, 27*8
    ld.d $r28, $sp, 28*8
    ld.d $r29, $sp, 29*8
    ld.d $r30, $sp, 30*8
    ld.d $r31, $sp, 31*8
.endm

.macro SAVE_REGS
    PUSH_GENERAL_REGS
    
    # ERA 
    csrrd $t0, 0x6
    st.d $t0, $sp, 32*8
    
    # PRMD
    csrrd $t0, 0x1
    st.d $t0, $sp, 31*8
    
    # ESTAT
    csrrd $t0, 0x5
    st.d $t0, $sp, 83*8
    
    # BADV
    csrrd $t0, 0x7
    st.d $t0, $sp, 84*8
    
    # trap_status = 1
    ori $t0, $zero, 1
    st.d $t0, $sp, 85*8
.endm

.macro RESTORE_REGS
    # ERA
    ld.d $t0, $sp, 32*8
    csrwr $t0, 0x6
    
    # PRMD
    ld.d $t0, $sp, 31*8
    csrwr $t0, 0x1
    
    POP_GENERAL_REGS
.endm

.section .text
.globl trap_vector_base
.globl trap_return1
.globl user_return1
.align 4

trap_vector_base:
    csrrd $t0, 0x1          # CSR_PRMD
    andi $t0, $t0, 0x3      # PLV field
    bnez $t0, user_trap
    b kernel_trap

kernel_trap:
    # 内核态异常处理
    addi.d $sp, $sp, -TRAPFRAME_SIZE
    SAVE_REGS
    or $a0, $sp, $zero
    ori $a1, $zero, 1       # has_trap = true
    ori $a2, $zero, 0       # from_user = false
    bl trampoline
    RESTORE_REGS
    ertn

user_trap:
    # 用户态异常处理
    SAVE_REGS
    or $a0, $sp, $zero
    ori $a1, $zero, 1       # has_trap = true
    ori $a2, $zero, 1       # from_user = true
    ld.d $sp, $sp, 33*8     # 切换到内核栈
    bl trampoline

trap_return1:
    or $sp, $a0, $zero
    ld.d $t0, $sp, 32*8     # ERA
    ld.d $t1, $sp, 31*8     # PRMD
    csrwr $t0, 0x6
    csrwr $t1, 0x1
    POP_GENERAL_REGS
    ld.d $sp, $sp, 33*8
    ertn

user_return1:
    or $sp, $a0, $zero
    ld.d $t0, $sp, 32*8     # ERA
    ld.d $t1, $sp, 31*8     # PRMD
    csrwr $t0, 0x6
    csrwr $t1, 0x1
    POP_GENERAL_REGS
    ld.d $sp, $sp, 2*8
    ertn