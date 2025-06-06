all:

	$(MAKE) -C os kernel MODE=release
	cp os/target/riscv64gc-unknown-none-elf/release/os ./kernel-rv
	touch kernel-la
	mv os/cargotemp os/.cargo

run:
	qemu-system-riscv64 \
   	  -machine virt \
	  -kernel kernel-rv \
	  -m 128M \
	  -nographic \
	  -smp 1 \
	  -bios default \
	  -drive file=os/sdcard-rv.img,if=none,format=raw,id=x0 \
	  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
	  -no-reboot \
	  -device virtio-net-device,netdev=net \
	  -netdev user,id=net \
	  -rtc base=utc


.PHONY: all run
