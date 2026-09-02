TARGET     := x86_64-unknown-none
BUILD      := build
ISO        := $(BUILD)/hexaos.iso
KERNEL_ELF := $(BUILD)/kernel.elf
RUST_LIB   := target/$(TARGET)/release/libhexa_kernel.a
QEMU       := qemu-system-x86_64 -m 256M

.PHONY: all iso test check run debug clean legacy-alpha-check ayo kernel-build

all: test check ayo

test:
	cargo test -p hexa-core

$(BUILD):
	mkdir -p $(BUILD)

$(BUILD)/boot.o: boot/boot.asm | $(BUILD)
	nasm -f elf64 $< -o $@

kernel-build:
	cargo build --release -p hexa-kernel --target $(TARGET)

$(KERNEL_ELF): linker.ld $(BUILD)/boot.o kernel-build
	ld -T linker.ld -o $@ $(BUILD)/boot.o $(RUST_LIB)

$(ISO): $(KERNEL_ELF) iso/boot/grub/grub.cfg | $(BUILD)
	cp $(KERNEL_ELF) iso/boot/kernel.elf
	grub-mkrescue -d /usr/lib/grub/i386-pc -o $@ iso/

iso: $(ISO)

check: $(ISO)
	rm -f $(BUILD)/serial.log
	timeout 20 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot > $(BUILD)/serial.log 2>&1 || true
	grep -q "HEXA_BOOT_OK" $(BUILD)/serial.log
	@echo ">>> HEXAOS SMOKE TEST PASSED <<<"

run: $(ISO)
	$(QEMU) -cdrom $(ISO) -serial stdio -no-reboot

debug: $(ISO)
	$(QEMU) -cdrom $(ISO) -display none -serial stdio -no-reboot -s -S

ayo:
	$(MAKE) -C ayo test build

legacy-alpha-check:
	$(MAKE) -C legacy/alpha32 clean all

clean:
	cargo clean
	$(MAKE) -C ayo clean
	rm -rf $(BUILD) iso/boot/kernel.elf
