GDBSERVER = localhost:1234
GDB = gdb-multiarch
GDBt = /home/ustc/qemu/gdb-14.2/build-riscv64/bin/riscv64-unknown-elf-gdb
all:


	cp -rT os/cargotemp os/.cargo	
	$(MAKE) -C os kernel MODE=release
	cp os/target/riscv64gc-unknown-none-elf/release/os ./kernel-rv
	touch kernel-la
run:
	qemu-system-riscv64 \
   	  -machine virt \
	  -kernel kernel-rv \
	  -m 1024M \
	  -nographic \
	  -smp 1 \
	  -bios default \
	  -drive file=os/sdcard-rv.img,if=none,format=raw,id=x0 \
	  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
	  -no-reboot \
	  -device virtio-net-device,netdev=net \
	  -netdev user,id=net \
	  -rtc base=utc
gdbserver:
	qemu-system-riscv64 \
   	  -machine virt \
	  -kernel kernel-rv \
	  -m 1024M \
	  -nographic \
	  -smp 1 \
	  -bios default \
	  -drive file=os/sdcard-rv.img,if=none,format=raw,id=x0 \
	  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
	  -no-reboot \
	  -device virtio-net-device,netdev=net \
	  -netdev user,id=net \
	  -rtc base=utc \
	  -s -S
all-debug:


	cp -rT os/cargotemp os/.cargo	
	$(MAKE) -C os kernel
	cp os/target/riscv64gc-unknown-none-elf/debug/os ./kernel-rv
	touch kernel-la

gdb:
	$(GDB) $(KERNEL) -ex "target remote $(GDBSERVER)" -ex "set arch riscv:rv64"
gdb2:
	$(GDBt) $(KERNEL) -ex "target remote $(GDBSERVER)" -ex "set arch riscv:rv64" 
.PHONY: all run gdbserver gdb
