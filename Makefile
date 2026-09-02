TARGET     := x86_64-unknown-none
BUILD      := build
ISO        := $(BUILD)/hexaos.iso
KERNEL_ELF := $(BUILD)/kernel.elf
RUST_LIB   := target/$(TARGET)/release/libhexa_kernel.a
QEMU       := qemu-system-x86_64 -m 256M

.PHONY: all iso test check run debug clean legacy-alpha-check run-alpha ayo kernel-build

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
	timeout 20 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-smoke-input.txt > $(BUILD)/serial.log 2>&1 || true
	grep -q "HEXA_BOOT_OK" $(BUILD)/serial.log
	grep -q "HEXA_SHELL_READY" $(BUILD)/serial.log
	grep -q "HEXA_COMMAND_OK help" $(BUILD)/serial.log
	grep -q "Ayo.*package" $(BUILD)/serial.log
	grep -q "Ayo --configured-by--> Root" $(BUILD)/serial.log
	grep -q "Browser --depends-on--> Ayo" $(BUILD)/serial.log
	grep -q "DIESE exact resolution" $(BUILD)/serial.log
	grep -q "Removed 'Browser' --depends-on--> 'Ayo'" $(BUILD)/serial.log
	grep -q "Reclaimed 'Scratch'" $(BUILD)/serial.log
	grep -q "Created and bound 'Browser'" $(BUILD)/serial.log
	grep -q "Granted Handle" $(BUILD)/serial.log
	grep -q "Handle #2 authorizes execute for requester 'Root'" $(BUILD)/serial.log
	grep -q "^42" $(BUILD)/serial.log
	grep -q "HexaOS Forms can carry structured state and revisions" $(BUILD)/serial.log
	grep -q "NotesBackup" $(BUILD)/serial.log
	grep -q "Created Dimension 'Development'" $(BUILD)/serial.log
	grep -q "Notes is now recoverable" $(BUILD)/serial.log
	grep -q "Notes is now active" $(BUILD)/serial.log
	@echo ">>> HEXAOS SMOKE TEST PASSED <<<"

run: $(ISO)
	$(QEMU) -cdrom $(ISO) -serial stdio -no-reboot

debug: $(ISO)
	$(QEMU) -cdrom $(ISO) -display none -serial stdio -no-reboot -s -S

ayo:
	$(MAKE) -C ayo test build

legacy-alpha-check:
	$(MAKE) -C legacy/alpha32 clean all

run-alpha:
	$(MAKE) -C legacy/alpha32 run

clean:
	cargo clean
	$(MAKE) -C ayo clean
	rm -rf $(BUILD) iso/boot/kernel.elf
