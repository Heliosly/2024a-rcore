.equ TRAPFRAME_SIZE, 688
.equ KSAVE_USP,  0x32

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
    st.d $r1,  $sp, 0*8
    st.d $r4,  $sp, 3*8
    st.d $r5,  $sp, 4*8
    st.d $r6,  $sp, 5*8
    st.d $r7,  $sp, 6*8
    st.d $r8,  $sp, 7*8
    st.d $r9,  $sp, 8*8
    st.d $r10, $sp, 9*8
    st.d $r11, $sp, 10*8
    st.d $r12, $sp, 11*8
    st.d $r13, $sp, 12*8
    st.d $r14, $sp, 13*8
    st.d $r15, $sp, 14*8
    st.d $r16, $sp, 15*8
    st.d $r17, $sp, 16*8
    st.d $r18, $sp, 17*8
    st.d $r19, $sp, 18*8
    st.d $r20, $sp, 19*8
    st.d $r22, $sp, 21*8
    st.d $r23, $sp, 22*8
    st.d $r24, $sp, 23*8
    st.d $r25, $sp, 24*8
    st.d $r26, $sp, 25*8
    st.d $r27, $sp, 26*8
    st.d $r28, $sp, 27*8
    st.d $r29, $sp, 28*8
    st.d $r30, $sp, 29*8
    st.d $r31, $sp, 30*8
.endm

.macro POP_GENERAL_REGS
    ld.d $r1,  $sp, 0*8
    ld.d $r4,  $sp, 3*8
    ld.d $r5,  $sp, 4*8
    ld.d $r6,  $sp, 5*8
    ld.d $r7,  $sp, 6*8
    ld.d $r8,  $sp, 7*8
    ld.d $r9,  $sp, 8*8
    ld.d $r10, $sp, 9*8
    ld.d $r11, $sp, 10*8
    ld.d $r12, $sp, 11*8
    ld.d $r13, $sp, 12*8
    ld.d $r14, $sp, 13*8
    ld.d $r15, $sp, 14*8
    ld.d $r16, $sp, 15*8
    ld.d $r17, $sp, 16*8
    ld.d $r18, $sp, 17*8
    ld.d $r19, $sp, 18*8
    ld.d $r20, $sp, 19*8
    ld.d $r22, $sp, 21*8
    ld.d $r23, $sp, 22*8
    ld.d $r24, $sp, 23*8
    ld.d $r25, $sp, 24*8
    ld.d $r26, $sp, 25*8
    ld.d $r27, $sp, 26*8
    ld.d $r28, $sp, 27*8
    ld.d $r29, $sp, 28*8
    ld.d $r30, $sp, 29*8
    ld.d $r31, $sp, 30*8
.endm

.macro SAVE_REGS
    PUSH_GENERAL_REGS
    csrrd   $t2, KSAVE_USP    # 读取 KSAVE_USP 到 $t0
    csrwr   $zero, KSAVE_USP  # 清零 KSAVE_USP
    st.d    $t2, $sp,  2*8 
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
.align 12
trap_vector_base:
    csrwr $sp, KSAVE_USP          # CSR_PRMD
    bnez $sp, user_trap
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
    SAVE_REGS                          # sp 已是 TrapFrame 指针，sscratch 保存了原 sp

    # 读取 sp（x3）和 tp（x4）保存的槽位
    ld.d   $t1, $sp, 20*8                # TrapFrame[20] -> 保存的是 sp
    ld.d   $t0, $sp, 1*8                # TrapFrame[1] -> 保存的是 tp

    # 把当前 gp（$r3）和 tp（$r4）写回 TrapFrame
    st.d   $r21, $sp, 20*8               # gp 写回 TrapFrame[2]
    st.d   $tp, $sp, 1*8                # tp 写回 TrapFrame[3]

    # 恢复 gp 和 tp
    or     $r21, $t1, $zero             # 恢复 gp
    or     $tp, $t0, $zero              # 恢复 tp

    # 设置 TrapFrame[85] = 1，表示任务优先处理 trap
    ori    $a0, $zero, 1
    st.d   $a0, $sp, 85*8

    # 设置 trampoline 参数
    or     $a0, $sp, $zero              # 参数1：TrapFrame 指针
    ori    $a1, $zero, 1                # 参数2：通过 trap 进入 trampoline
    ori    $a2, $zero, 1                # 参数3：由用户态进入

    # 切换内核栈
    ld.d   $sp, $sp, 33*8               # 从 TrapFrame[38] 恢复内核栈顶指针

    bl     trampoline                   # 不会返回，最终从 trampoline 走 trap_return

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
    # 参数 a0 是 TrapContext 指针，设 sp = a0
    or      $sp, $a0, $zero

    # 交换 gp <-> x3 (TrapContext.x[20])
    ld.d    $t0, $sp, 20*8
    st.d    $r21, $sp, 20*8
    or      $r21, $t0, $zero

    # 交换 tp <-> x4 (TrapContext.x[1])
    ld.d    $t0, $sp, 1*8
    st.d    $tp, $sp, 1*8
    or      $tp, $t0, $zero

    # 存一份 TrapContext 指针到 KSAVE_USP，方便 trap handler 使用
    csrwr     $a0 , KSAVE_USP
    # 恢复 era（相当于 sepc）和 prmd（相当于 sstatus）
    ld.d    $t0, $sp, 32*8      # TrapContext.sepc
    ld.d    $t1, $sp, 31*8      # TrapContext.sstatus
    csrwr    $t0 , 0x6      # era
    csrwr     $t1 , 0x1     # prmd

    POP_GENERAL_REGS

    # 恢复用户态 sp（x3）
    ld.d $sp, $sp, 2*8
    ertn
