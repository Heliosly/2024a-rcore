
TARGET := riscv64gc-unknown-none-elf
MODE ?= debug
KERNEL_ELF := target/$(TARGET)/$(MODE)/os
KERNEL_BIN := $(KERNEL_ELF).bin
DISASM_TMP := target/$(TARGET)/$(MODE)/asm
FS_IMG := ./sdcard-rv.img

#FS_IMG := ./fs.img

APPS := ../user/src/bin/*
OFFLINE :=

# BOARD
BOARD := qemu
SBI ?= rustsbi
BOOTLOADER := ../bootloader/$(SBI)-$(BOARD).bin

# KERNEL ENTRY
KERNEL_ENTRY_PA := 0x80200000

# Binutils
OBJDUMP := rust-objdump --arch-name=riscv64
OBJCOPY := rust-objcopy --binary-architecture=riscv64

# GDB Dashboard 支持
GDB_PATH := /home/ustc/qemu/gdb-14.2/build-riscv64/bin/riscv64-unknown-elf-gdb
gdb := $(GDB_PATH) 

# Building mode argument
ifeq ($(MODE), release)
	MODE_ARG := --release
endif

# 测试参数
CHAPTER ?= $(shell git rev-parse --abbrev-ref HEAD | sed -E 's/ch([0-9])/\1/')
TEST ?= 6
BASE ?= 1
# Disassembly
DISASM ?= -x

build: env $(KERNEL_BIN) 

env:
ifeq ($(OFFLINE),1)
	(rustup target list | grep "riscv64gc-unknown-none-elf (installed)") || rustup target add $(TARGET)
	cargo install cargo-binutils
	rustup component add rust-src
	rustup component add llvm-tools-preview
endif

$(KERNEL_BIN): kernel
	@$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $@
kernel:
	@echo Platform: $(BOARD)
	@cargo build $(MODE_ARG) --offline

clean:
	@cargo clean

disasm: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) | less

disasm-vim: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) > $(DISASM_TMP)
	@vim $(DISASM_TMP)
	@rm $(DISASM_TMP)

run: run-inner

run-inner: build
	@qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-m 1024M \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) \
		-drive file=$(FS_IMG),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0\
		-drive file=disk.img,if=none,format=raw,id=x1 -device virtio-blk-device,drive=x1,bus=virtio-mmio-bus.1

gdb: build
	@tmux new-session -d \
		"qemu-system-riscv64 -machine virt -nographic -m 1024M -bios $(BOOTLOADER) -device loader,file=$(KERNEL_ELF),addr=$(KERNEL_ENTRY_PA) -s -S" && \
		tmux split-window -h \"$(gdb) -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'\" && \
		tmux swap-pane -U && \
		tmux -2 attach-session -d

gdbserver: build
	@qemu-system-riscv64 -machine virt -nographic -m 1024M -bios $(BOOTLOADER) -device loader,file=$(KERNEL_ELF),addr=$(KERNEL_ENTRY_PA) -s -S \
		-drive file=$(FS_IMG),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-drive file=disk.img,if=none,format=raw,id=x1\
		-device virtio-blk-device,drive=x1,bus=virtio-mmio-bus.1\
		

gdbclient:
	@gdb -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'


.PHONY: build env kernel clean disasm disasm-vim run-inner fs-img gdbserver gdbclient
