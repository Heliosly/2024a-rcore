OUTPUT_ARCH(%ARCH%)
ENTRY(_start)

BASE_ADDRESS = %KERNEL_BASE%;

SECTIONS
{
    . = BASE_ADDRESS;
    _skernel = .;

    .text ALIGN(16K): {
        *(.multiboot .multiboot.*)
        stext = .;
        *(.text.entry)
        *(.text .text.*)
        etext = .;
    }

    .rodata ALIGN(16K): {
        srodata = .;
        *(.rodata .rodata.*)
        erodata = .;
    }

    .got : {
        *(.got .got.*)
    }

    .data ALIGN(16K): {
        _sdata = .;
        . = ALIGN(16K);
        *(.data.boot_page_table)

        *(.data .data.*)
        *(.sdata .sdata.*)
        _edata = .;
    }

    .bss ALIGN(16K): {
        _load_end = .;
        *(.bss.bstack .bss.bstack.*)
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _ebss = .;
    }

    PROVIDE(_end = .);
    /DISCARD/ : {
        *(.comment) *(.gnu*) *(.note*) *(.eh_frame*)
    }
}
