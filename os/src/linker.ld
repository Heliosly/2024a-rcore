OUTPUT_ARCH(riscv)
ENTRY(_start)
/* BASE_ADDRESS = 0x80200000; */
BASE_ADDRESS = 0xffffffc080200000;

SECTIONS
{
    . = BASE_ADDRESS;
    skernel = .;

    stext = .;
    .text : AT(0x80200000) {
    *(.text.entry)
        . = ALIGN(4);           /* 强制4字节对齐 */
        *(.text.trap_entries)   /* 存放 trap_from_kernel 和 __trap_from_user */
    . = ALIGN(4K);
        strampoline = .;
        . = ALIGN(4K);
        *(.text .text.*)
}
    
    
    . = ALIGN(4K);
    etext = .;
    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    . = ALIGN(4K);
    erodata = .;
    sdata = .;
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    . = ALIGN(4K);
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

    /DISCARD/ : {
        *(.eh_frame)
    }
}