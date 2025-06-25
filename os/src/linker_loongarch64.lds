OUTPUT_ARCH(loongarch64)
ENTRY(_start)

BASE_ADDRESS = 0x9000000080200000;


SECTIONS
{
    . = BASE_ADDRESS;
    _skernel = .;

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
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    
    . = ALIGN(4K);
    _ebss = .;
    _ekernel = .;
    ekernel = .;

    


 PROVIDE(_end = .);
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