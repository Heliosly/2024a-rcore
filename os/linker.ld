OUTPUT_ARCH(%ARCH%)
ENTRY(_start)

BASE_ADDRESS = %KERNEL_BASE%;

SECTIONS
{
    . = BASE_ADDRESS;
    _skernel = .;
    skernel = .;

    /* 文本段：包含入口点、陷阱处理、信号蹦床和常规代码 */
    .text ALIGN(4K): {
        *(.multiboot .multiboot.*)    /* 多引导头 */
        stext = .;
        *(.text.entry)                /* 内核入口点 */
        
        /* 陷阱处理入口（强制4字节对齐） */
        . = ALIGN(4);
        *(.text.trap_entries)         /* trap_from_kernel 和 __trap_from_user */
        
        /* 信号蹦床（4K对齐）*/
        . = ALIGN(4K);
        PROVIDE(strampoline = .);
        *(.text.signal_trampoline)
        . = ALIGN(4K);
        PROVIDE(etrampoline = .);
        
        /* 常规代码 */
        *(.text .text.*)
        etext = .;
    }

    /* 只读数据段 */
    . = ALIGN(4K);
    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    . = ALIGN(4K);
    erodata = .;

    /* 全局偏移表（位置无关代码必需）*/
    .got ALIGN(4K): {
        *(.got .got.*)
    }

    /* 数据段：包含启动页表和其他数据 */
    . = ALIGN(4K);
    sdata = .;
    _sdata = .;
    .data : {
        /* 启动页表（4K对齐）*/
        . = ALIGN(4K);
        *(.data.boot_page_table)
        
        /* 常规数据 */
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    
    /* 线程局部存储数据 */
    .tdata ALIGN(0x10): {
        _stdata = .;
        *(.tdata .tdata.*)
        _etdata = .;
    }
    
    /* 线程局部存储BSS */
    .tbss ALIGN(0x10): {
        _stbss = .;
        *(.tbss .tbss.*)
        *(.tcommon)
        _etbss = .;
    }
    . = ALIGN(4K);
    edata = .;
    _edata = .;

    /* Per-CPU数据（多核支持）*/
    . = ALIGN(4K);
    spercpu = .;
    .percpu : {
        _percpu_load_start = .;
        *(.percpu .percpu.*)
        _percpu_load_end = .;
        . = ALIGN(64);
    }
    epercpu = .;

    /* BSS段：栈放在最前防止覆盖 */
    . = ALIGN(4K);
    sbss_with_stack = .;
    .bss : {
        /* 启动栈放在BSS开头 */
        *(.bss.bstack .bss.bstack.*)
        
        sbss = .;
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    . = ALIGN(4K);
    ebss = .;
    _ebss = .;
    ekernel = .;

    /* 结束标志 */
    _ekernel = .;
    PROVIDE(_end = .);
    
    /* 丢弃无用段 */
    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.gnu*)
        *(.note*)
    }
}