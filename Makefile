TARGET     := x86_64-unknown-none
BUILD      := build
RUNTIME    := runtime
ISO        := $(BUILD)/hexaos.iso
KERNEL_ELF := $(BUILD)/kernel.elf
RUST_LIB   := target/$(TARGET)/release/libhexa_kernel.a
STATE_IMG  := $(RUNTIME)/expos-state.img
STATE_SIZE := 4M
QEMU       := qemu-system-x86_64 -machine pc -cpu max -m 256M -vga std -global VGA.vgamem_mb=16 -netdev user,id=net0 -device rtl8139,netdev=net0

.PHONY: all iso test check display-check session-check network-check internet-check https-check search-check persistence-check run debug clean legacy-alpha-check run-alpha ayo go-sdk kernel-build

all: test check display-check session-check network-check persistence-check ayo go-sdk

test:
	cargo test --workspace

$(BUILD):
	mkdir -p $(BUILD)

$(STATE_IMG):
	mkdir -p $(RUNTIME)
	truncate -s $(STATE_SIZE) $@

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
	rm -f $(BUILD)/check-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/check-state.img
	set +e; timeout 30 $(QEMU) -drive file=$(BUILD)/check-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-smoke-input.txt > $(BUILD)/serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
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
	grep -q "TSC frequency: .* Hz" $(BUILD)/serial.log
	grep -q "clock source:" $(BUILD)/serial.log
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
	! grep -q "nightowl" $(BUILD)/serial.log
	! grep -q "starlight" $(BUILD)/serial.log
	@echo ">>> EXPOS SMOKE TEST PASSED <<<"

display-check: $(ISO)
	rm -f $(BUILD)/display-serial.log
	rm -f $(BUILD)/display-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/display-state.img
	set +e; timeout 30 $(QEMU) -drive file=$(BUILD)/display-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-display-input.txt > $(BUILD)/display-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "framebuffer: 1920x1080 XRGB8888 scanout available=true" $(BUILD)/display-serial.log
	grep -q "HEXA_BOOT_MODE graphical" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_MODE width=1920 height=1080 bpp=32 stride=7680 bytes=8294400" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_MODE .*pageflip=true" $(BUILD)/display-serial.log
	grep -q "HEXA_PRESENTATION_READY rate=60 Hz vsync=true pageflip=true" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_READY surfaces=11 commit=11" $(BUILD)/display-serial.log
	grep -q "HEXA_DESKTOP_EMPTY open_apps=0 pinned_apps=0" $(BUILD)/display-serial.log
	grep -q "HEXA_MOUSE_READY enabled=true" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_OPENED SETTINGS" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=theme value=Graphite" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=resolution value=480p" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=resolution value=720p" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=refresh-rate value=75 Hz" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=refresh-rate value=120 Hz" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=refresh-rate value=144 Hz" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=vsync value=off" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=vsync value=on" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_MODE width=1280 height=720 bpp=32 stride=5120 bytes=3686400 preset=720p" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=taskbar value=off" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=network value=off" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_CHANGED key=network value=on" $(BUILD)/display-serial.log
	grep -q "HEXA_SETTING_DENIED key=bluetooth error=BusUnsupported" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_OPENED TERMINAL" $(BUILD)/display-serial.log
	grep -q "HEXA_TERMINAL_COMMAND name=help" $(BUILD)/display-serial.log
	grep -q "HEXA_TERMINAL_COMMAND name=status" $(BUILD)/display-serial.log
	grep -q "HEXA_TERMINAL_COMMAND name=storage" $(BUILD)/display-serial.log
	grep -q "HEXA_TERMINAL_COMMAND name=theme" $(BUILD)/display-serial.log
	grep -q "HEXA_TERMINAL_COMMAND name=ps" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_CLOSED TERMINAL" $(BUILD)/display-serial.log
	grep -q "HEXA_APP_REOPENED TERMINAL" $(BUILD)/display-serial.log
	grep -Eq "HEXA_PRESENTATION_STATS frames=[1-9][0-9]* missed=[0-9]+ idle=[0-9]+ vblank_timeouts=0" $(BUILD)/display-serial.log
	grep -q "HEXA_DISPLAY_CLOSED" $(BUILD)/display-serial.log
	grep -q "HEXA_COMMAND_OK desktop" $(BUILD)/display-serial.log
	grep -q "HEXA_PRESENTATION_READY rate=144 Hz vsync=true pageflip=true" $(BUILD)/display-serial.log
	grep -q "ExpOS Go ABI v1" $(BUILD)/display-serial.log
	@echo ">>> EXPOS DISPLAY TEST PASSED <<<"

session-check: $(ISO)
	rm -f $(BUILD)/guest-serial.log
	rm -f $(BUILD)/guest-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/guest-state.img
	set +e; timeout 30 $(QEMU) -drive file=$(BUILD)/guest-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-guest-input.txt > $(BUILD)/guest-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_LOGIN_OK user=guest" $(BUILD)/guest-serial.log
	grep -q "guest (Guest authority)" $(BUILD)/guest-serial.log
	grep -q "DIESE denied 'mkform' for Guest authority" $(BUILD)/guest-serial.log
	! grep -q "Created and bound 'Forbidden'" $(BUILD)/guest-serial.log
	grep -q "HEXA_COMMAND_OK shutdown" $(BUILD)/guest-serial.log
	@echo ">>> EXPOS SESSION AUTHORITY TEST PASSED <<<"

network-check: $(ISO)
	rm -f $(BUILD)/network-serial.log
	rm -f $(BUILD)/network-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/network-state.img
	python3 -m http.server 18080 --bind 127.0.0.1 --directory tests > $(BUILD)/http-fixture.log 2>&1 & fixture_pid=$$!; \
	trap 'kill $$fixture_pid 2>/dev/null || true' EXIT; \
	set +e; timeout 35 $(QEMU) -drive file=$(BUILD)/network-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-network-input.txt > $(BUILD)/network-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_NET_READY driver=rtl8139" $(BUILD)/network-serial.log
	grep -q "ether0  up" $(BUILD)/network-serial.log
	grep -q "HEXA_HTTP_OK status=200 bytes=38 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "ExpOS native TCP and HTTP are online." $(BUILD)/network-serial.log
	grep -q "HEXA_BROWSER_ENGINE nodes=4 css_rules=5 scripts=1 executed=1 rejected=0 handlers=1" $(BUILD)/network-serial.log
	grep -q "HEXA_BROWSER_HTTP_OK status=200 bytes=786 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "HEXA_BROWSER_REDIRECT status=301 hop=1" $(BUILD)/network-serial.log
	grep -q "HEXA_BROWSER_HTTP_OK status=200 bytes=134 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "HEXA_PING_REPLY address=10.0.2.2 sequence=1" $(BUILD)/network-serial.log
	grep -q "HEXA_PING_SUMMARY sent=160 received=160" $(BUILD)/network-serial.log
	! grep -q "HEXA_PING_ERROR" $(BUILD)/network-serial.log
	! grep -q "HEXA_HTTP_ERROR" $(BUILD)/network-serial.log
	! grep -q "HEXA_BROWSER_HTTP_ERROR" $(BUILD)/network-serial.log
	grep -q "rtl8139-poll up" $(BUILD)/network-serial.log
	@echo ">>> EXPOS NATIVE NETWORK TEST PASSED <<<"

internet-check: $(ISO)
	rm -f $(BUILD)/internet-serial.log
	rm -f $(BUILD)/internet-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/internet-state.img
	set +e; timeout 35 $(QEMU) -drive file=$(BUILD)/internet-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-internet-input.txt > $(BUILD)/internet-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_DNS_OK host=example.com address=" $(BUILD)/internet-serial.log
	grep -q "HEXA_HTTP_OK status=" $(BUILD)/internet-serial.log
	! grep -q "HEXA_DNS_ERROR" $(BUILD)/internet-serial.log
	! grep -q "HEXA_HTTP_ERROR" $(BUILD)/internet-serial.log
	@echo ">>> EXPOS LIVE INTERNET TEST PASSED <<<"

https-check: $(ISO)
	rm -f $(BUILD)/https-serial.log
	rm -f $(BUILD)/https-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/https-state.img
	set +e; timeout 60 $(QEMU) -drive file=$(BUILD)/https-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-https-input.txt > $(BUILD)/https-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_TLS_VERIFIED host=www.youtube.com version=1.3" $(BUILD)/https-serial.log
	grep -q "HEXA_HTTP_OK status=" $(BUILD)/https-serial.log
	grep -q "HEXA_BROWSER_HTTP_OK status=200" $(BUILD)/https-serial.log
	! grep -q "HEXA_HTTP_ERROR" $(BUILD)/https-serial.log
	! grep -q "HEXA_BROWSER_HTTP_ERROR" $(BUILD)/https-serial.log
	@echo ">>> EXPOS VERIFIED HTTPS TEST PASSED <<<"

search-check: $(ISO)
	rm -f $(BUILD)/search-serial.log
	rm -f $(BUILD)/search-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/search-state.img
	set +e; timeout 60 $(QEMU) -drive file=$(BUILD)/search-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-search-input.txt > $(BUILD)/search-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_BROWSER_SEARCH query_bytes=23 .*provider=duckduckgo-html" $(BUILD)/search-serial.log
	grep -q "HEXA_TLS_VERIFIED host=duckduckgo.com version=1.3" $(BUILD)/search-serial.log
	grep -Eq 'HEXA_SEARCH_RESULTS count=[1-8][[:space:]]*$$' $(BUILD)/search-serial.log
	grep -q "HEXA_BROWSER_HTTP_OK status=200" $(BUILD)/search-serial.log
	! grep -q "HEXA_BROWSER_HTTP_ERROR" $(BUILD)/search-serial.log
	@echo ">>> EXPOS DUCKDUCKGO HTML SEARCH TEST PASSED <<<"

persistence-check: $(ISO)
	rm -f $(BUILD)/persistence-state.img $(BUILD)/persistence-write.log $(BUILD)/persistence-read.log
	truncate -s $(STATE_SIZE) $(BUILD)/persistence-state.img
	set +e; timeout 40 $(QEMU) -drive file=$(BUILD)/persistence-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-persistence-write-input.txt > $(BUILD)/persistence-write.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	set +e; timeout 40 $(QEMU) -drive file=$(BUILD)/persistence-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-persistence-read-input.txt > $(BUILD)/persistence-read.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "HEXA_USER_CREATED keeper" $(BUILD)/persistence-write.log
	grep -q "HEXA_SETTING_CHANGED key=theme value=Graphite" $(BUILD)/persistence-write.log
	grep -q "HEXA_SETTING_CHANGED key=resolution value=480p" $(BUILD)/persistence-write.log
	grep -q "HEXA_SETTING_CHANGED key=refresh-rate value=144 Hz" $(BUILD)/persistence-write.log
	grep -q "HEXA_SETTING_CHANGED key=vsync value=off" $(BUILD)/persistence-write.log
	grep -q "HEXA_STATE_COMMIT generation=" $(BUILD)/persistence-write.log
	! grep -q "violetmemory" $(BUILD)/persistence-write.log
	! LC_ALL=C grep -a -q "violetmemory" $(BUILD)/persistence-state.img
	grep -q "HEXA_STATE_READY generation=.*loaded=true" $(BUILD)/persistence-read.log
	grep -q "HEXA_ACCOUNTS_READY source=disk persisted=true" $(BUILD)/persistence-read.log
	grep -q "HEXA_LOGIN_OK user=keeper" $(BUILD)/persistence-read.log
	grep -q "keeper (Power authority)" $(BUILD)/persistence-read.log
	grep -q "HEXA_DISPLAY_MODE width=640 height=480 bpp=32 stride=2560 bytes=1228800 preset=480p" $(BUILD)/persistence-read.log
	grep -q "HEXA_DESKTOP_PREFS theme=Graphite" $(BUILD)/persistence-read.log
	grep -q "HEXA_PRESENTATION_READY rate=144 Hz vsync=false pageflip=true" $(BUILD)/persistence-read.log
	! grep -q "violetmemory" $(BUILD)/persistence-read.log
	grep -q "HEXA_COMMAND_OK shutdown" $(BUILD)/persistence-read.log
	@echo ">>> EXPOS PERSISTENCE TEST PASSED <<<"

run: $(ISO) $(STATE_IMG)
	$(QEMU) -drive file=$(STATE_IMG),format=raw,if=ide,index=0 -boot once=d -cdrom $(ISO) -serial stdio -no-reboot

debug: $(ISO) $(STATE_IMG)
	$(QEMU) -drive file=$(STATE_IMG),format=raw,if=ide,index=0 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot -s -S

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
