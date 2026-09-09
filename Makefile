TARGET     := x86_64-unknown-none
BUILD      := build
ISO        := $(BUILD)/hexaos.iso
KERNEL_ELF := $(BUILD)/kernel.elf
RUST_LIB   := target/$(TARGET)/release/libhexa_kernel.a
QEMU       := qemu-system-x86_64 -machine pc -m 256M -vga std -global VGA.vgamem_mb=16 -netdev user,id=net0 -device rtl8139,netdev=net0

.PHONY: all iso test check display-check session-check network-check internet-check run debug clean legacy-alpha-check run-alpha ayo go-sdk kernel-build

all: test check display-check session-check network-check ayo go-sdk

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
	grep -q "HEXA_BOOT_MODE console" $(BUILD)/serial.log
	grep -q "HEXA_LOGIN_OK user=operator" $(BUILD)/serial.log
	grep -q "HEXA_SHELL_READY" $(BUILD)/serial.log
	grep -q "HEXA_COMMAND_OK help" $(BUILD)/serial.log
	grep -q "HEXA_USER_CREATED artist" $(BUILD)/serial.log
	grep -q "artist (Power authority)" $(BUILD)/serial.log
	grep -q "HEXA_PASSWORD_CHANGED artist" $(BUILD)/serial.log
	grep -q "HEXA_USER_DELETED artist" $(BUILD)/serial.log
	grep -q "KERNEL FEATURE MATRIX" $(BUILD)/serial.log
	grep -q "Ayo.*package" $(BUILD)/serial.log
	grep -q "HexaDisplay.*service" $(BUILD)/serial.log
	grep -q "GoABI.*interface" $(BUILD)/serial.log
	grep -q "Ayo --configured-by--> Root" $(BUILD)/serial.log
	grep -q "DemoBrowser --depends-on--> Ayo" $(BUILD)/serial.log
	grep -q "DIESE exact resolution" $(BUILD)/serial.log
	grep -q "Removed 'DemoBrowser' --depends-on--> 'Ayo'" $(BUILD)/serial.log
	grep -q "Reclaimed 'Scratch'" $(BUILD)/serial.log
	grep -q "Created and bound 'DemoBrowser'" $(BUILD)/serial.log
	grep -q "Granted Handle" $(BUILD)/serial.log
	grep -q "Handle #3 authorizes execute for requester 'Root'" $(BUILD)/serial.log
	grep -q "^42" $(BUILD)/serial.log
	grep -q "ExpOS Forms can carry structured state and revisions" $(BUILD)/serial.log
	grep -q "NotesBackup" $(BUILD)/serial.log
	grep -q "Created Dimension 'Development'" $(BUILD)/serial.log
	grep -q "Notes is now recoverable" $(BUILD)/serial.log
	grep -q "Notes is now active" $(BUILD)/serial.log
	@echo ">>> EXPOS SMOKE TEST PASSED <<<"

display-check: $(ISO)
	rm -f $(BUILD)/display-serial.log
	timeout 20 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-display-input.txt > $(BUILD)/display-serial.log 2>&1 || true
	grep -q "framebuffer: 1920x1080 XRGB8888 scanout available=true" $(BUILD)/display-serial.log
	grep -q "HEXA_BOOT_MODE graphical" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_MODE width=1920 height=1080 bpp=32 stride=7680 bytes=8294400" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_READY surfaces=11 commit=11" $(BUILD)/display-serial.log
	grep -q "HEXA_DESKTOP_EMPTY open_apps=0 pinned_apps=0" $(BUILD)/display-serial.log
	grep -q "HEXA_MOUSE_READY enabled=true" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_OPENED SETTINGS" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=accent value=Ocean" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=taskbar value=off" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=network value=off" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=network value=on" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_DENIED key=bluetooth error=BusUnsupported" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_OPENED TERMINAL" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_CLOSED TERMINAL" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_REOPENED TERMINAL" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_CLOSED" $(BUILD)/display-serial.log
	grep -q "HEXA_COMMAND_OK desktop" $(BUILD)/display-serial.log
	grep -q "ExpOS Go ABI v1" $(BUILD)/display-serial.log
	@echo ">>> EXPOS DISPLAY TEST PASSED <<<"

session-check: $(ISO)
	rm -f $(BUILD)/guest-serial.log
	timeout 20 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-guest-input.txt > $(BUILD)/guest-serial.log 2>&1 || true
	grep -q "HEXA_LOGIN_OK user=guest" $(BUILD)/guest-serial.log
	grep -q "guest (Guest authority)" $(BUILD)/guest-serial.log
	grep -q "DIESE denied 'mkform' for Guest authority" $(BUILD)/guest-serial.log
	! grep -q "Created and bound 'Forbidden'" $(BUILD)/guest-serial.log
	@echo ">>> EXPOS SESSION AUTHORITY TEST PASSED <<<"

network-check: $(ISO)
	rm -f $(BUILD)/network-serial.log
	python3 -m http.server 18080 --bind 127.0.0.1 --directory tests > $(BUILD)/http-fixture.log 2>&1 & fixture_pid=$$!; \
	trap 'kill $$fixture_pid 2>/dev/null || true' EXIT; \
	timeout 30 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-network-input.txt > $(BUILD)/network-serial.log 2>&1 || true
	grep -q "HEXA_NET_READY driver=rtl8139" $(BUILD)/network-serial.log
	grep -q "ether0  up" $(BUILD)/network-serial.log
	grep -q "HEXA_HTTP_OK status=200 bytes=38 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "ExpOS native TCP and HTTP are online." $(BUILD)/network-serial.log
	grep -q "HEXA_BROWSER_HTTP_OK status=200 bytes=38 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "HEXA_PING_REPLY address=10.0.2.2 sequence=1" $(BUILD)/network-serial.log
	grep -q "HEXA_PING_SUMMARY sent=160 received=160" $(BUILD)/network-serial.log
	! grep -q "HEXA_PING_ERROR" $(BUILD)/network-serial.log
	! grep -q "HEXA_HTTP_ERROR" $(BUILD)/network-serial.log
	grep -q "rtl8139-poll up" $(BUILD)/network-serial.log
	@echo ">>> EXPOS NATIVE NETWORK TEST PASSED <<<"

internet-check: $(ISO)
	rm -f $(BUILD)/internet-serial.log
	timeout 30 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-internet-input.txt > $(BUILD)/internet-serial.log 2>&1 || true
	grep -q "HEXA_DNS_OK host=example.com address=" $(BUILD)/internet-serial.log
	grep -q "HEXA_HTTP_OK status=" $(BUILD)/internet-serial.log
	! grep -q "HEXA_DNS_ERROR" $(BUILD)/internet-serial.log
	! grep -q "HEXA_HTTP_ERROR" $(BUILD)/internet-serial.log
	@echo ">>> EXPOS LIVE INTERNET TEST PASSED <<<"

run: $(ISO)
	$(QEMU) -cdrom $(ISO) -serial stdio -no-reboot

debug: $(ISO)
	$(QEMU) -cdrom $(ISO) -display none -serial stdio -no-reboot -s -S

ayo:
	$(MAKE) -C ayo test build

go-sdk:
	GOCACHE=/tmp/hexaos-go-sdk-cache go -C sdk/go test ./...

legacy-alpha-check:
	$(MAKE) -C legacy/alpha32 clean all

run-alpha:
	$(MAKE) -C legacy/alpha32 run

clean:
	cargo clean
	$(MAKE) -C ayo clean
	rm -rf $(BUILD) iso/boot/kernel.elf
