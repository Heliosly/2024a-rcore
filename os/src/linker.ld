OUTPUT_ARCH(%ARCH%)
ENTRY(_start)

BASE_ADDRESS = %KERNEL_BASE%;


SECTIONS
{
    . = BASE_ADDRESS;
    skernel = .;

    /* 代码段 */
    stext = .;
    .text : AT(BASE_ADDRESS) {
        *(.text.entry)
        . = ALIGN(4);
        *(.text.trap_entries)
        . = ALIGN(4K);
        
        /* 信号蹦床代码 */
        PROVIDE(strampoline = .);
        *(.text.signal_trampoline)
        . = ALIGN(4K);
        PROVIDE(etrampoline = .);
        
        /* 普通代码 */
        *(.text .text.*)
    }
    
    . = ALIGN(4K);  
    etext = .;
    
    /* 只读数据段 */
    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    
    . = ALIGN(4K);
    erodata = .;
    
    /* 可写数据段 */
    sdata = .;
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    
    /* 线程局部存储数据 */
    .tdata : ALIGN(0x10) {
        _stdata = .;
        *(.tdata .tdata.*)
        _etdata = .;
    }
    
    /* 线程局部存储BSS */
    .tbss : ALIGN(0x10) {
        _stbss = .;
        *(.tbss .tbss.*)
        *(.tcommon)
        _etbss = .;
    }
    
    . = ALIGN(4K);
    
    /* 每CPU数据段 */
    spercpu = .;
    .percpu : {
        _percpu_load_start = .;
        *(.percpu .percpu.*)
        _percpu_load_end = .;
        . = ALIGN(64);
    }
    epercpu = .;
    
    . = ALIGN(4K);
    edata = .;
    
    /* BSS段 */
    sbss_with_stack = .;
    .bss : {
        *(.bss.stack)
        sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    
    . = ALIGN(4K);
    ebss = .;
    ekernel = .;
    
    /* LoongArch64 特定段 */
    .got : {
        *(.got)
        *(.got.plt)
    }
    
    .plt : {
        *(.plt)
    }
    

    .debug_info     0 : { *(.debug_info) }
    .debug_abbrev   0 : { *(.debug_abbrev) }
    .debug_line     0 : { *(.debug_line) }
    .debug_frame    0 : { *(.debug_frame) }
    .debug_str      0 : { *(.debug_str) }
    .debug_loc      0 : { *(.debug_loc) }
    .debug_ranges   0 : { *(.debug_ranges) }
    
    /* 丢弃不需要的段 */
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note.gnu.build-id)
        *(.comment)
        *(.gnu.hash)
        *(.dynsym)
        *(.dynstr)
        *(.hash)
    }
}