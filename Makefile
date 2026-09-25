TARGET     := x86_64-unknown-none
BUILD      := build
RUNTIME    := runtime
ISO        := $(BUILD)/expos.iso
KERNEL_ELF := $(BUILD)/kernel.elf
RUST_LIB   := target/$(TARGET)/release/libexpos_kernel.a
STATE_IMG  := $(RUNTIME)/expos-state.img
STATE_SIZE := 4M
QEMU       := qemu-system-x86_64 -machine pc -cpu max -m 256M -vga std -global VGA.vgamem_mb=16 -netdev user,id=net0 -device rtl8139,netdev=net0
UEFI_DIR   := $(BUILD)/esp
UEFI_APP   := $(UEFI_DIR)/EFI/BOOT/BOOTX64.EFI
UEFI_BOOT_IMG := $(BUILD)/uefi-boot.img
GENESIS_DIR := $(BUILD)/genesis
GENESIS_RUNTIME := $(GENESIS_DIR)/runtime-BOOTX64.EFI
GENESIS_RUST_LIB := target/genesis/$(TARGET)/release/libexpos_kernel.a
GENESIS_KERNEL := $(GENESIS_DIR)/kernel.elf
GENESIS_PAYLOAD_OBJ := $(GENESIS_DIR)/uefi-payload.obj
GENESIS_APP := $(GENESIS_DIR)/BOOTX64.EFI
GENESIS_EFI_IMG := $(GENESIS_DIR)/efiboot.img
GENESIS_ISO := $(BUILD)/ExpOS-0.9-x86_64.iso
OVMF_CODE  ?= /usr/share/edk2/x64/OVMF_CODE.4m.fd
OVMF_VARS  ?= /usr/share/edk2/x64/OVMF_VARS.4m.fd

.PHONY: all iso genesis-iso genesis-check test check display-check session-check network-check internet-check https-check search-check persistence-check run debug clean legacy-alpha-check run-alpha ayo sdk rust-sdk go-sdk c-sdk python-sdk sdk-cli-check kernel-build genesis-kernel-build

all: test check display-check session-check network-check persistence-check ayo sdk python-runtime-check python-check budget-check uefi-check bootmode-check startup-check

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
	cargo build --release -p expos-kernel --target $(TARGET)

$(KERNEL_ELF): linker.ld $(BUILD)/boot.o kernel-build
	ld -T linker.ld -o $@ $(BUILD)/boot.o $(RUST_LIB)

$(ISO): $(KERNEL_ELF) iso/boot/grub/grub.cfg | $(BUILD)
	cp $(KERNEL_ELF) iso/boot/kernel.elf
	grub-mkrescue -d /usr/lib/grub/i386-pc -o $@ iso/

iso: $(ISO)

# Genesis installation media is a separate UEFI build. It embeds the normal
# native runtime application, then writes that application to the selected
# disk's standards-based EFI/BOOT fallback path.
$(GENESIS_RUNTIME): $(UEFI_APP)
	mkdir -p $(dir $@)
	cp $< $@

genesis-kernel-build: $(GENESIS_RUNTIME)
	CARGO_TARGET_DIR=target/genesis cargo build --release -p expos-kernel --target $(TARGET) --features genesis-installer

$(GENESIS_KERNEL): linker.ld $(BUILD)/boot.o genesis-kernel-build
	ld --defsym=KERNEL_LOAD_BASE=0x2000000 -T linker.ld -e _uefi_start -o $@ $(BUILD)/boot.o $(GENESIS_RUST_LIB)

$(GENESIS_PAYLOAD_OBJ): boot/uefi/genesis-payload.asm $(GENESIS_KERNEL)
	nasm -f win64 $< -o $@

$(GENESIS_APP): $(BUILD)/uefi-loader.obj $(GENESIS_PAYLOAD_OBJ)
	ld -mi386pep --subsystem 10 --entry efi_main --image-base 0 --enable-reloc-section -o $@ $^

$(GENESIS_EFI_IMG): $(GENESIS_APP) tools/make-efi-fat.py
	python3 tools/make-efi-fat.py $(GENESIS_APP) $@

$(GENESIS_ISO): $(GENESIS_EFI_IMG)
	mkdir -p $(GENESIS_DIR)/iso/boot $(GENESIS_DIR)/iso/EFI/BOOT
	cp $(GENESIS_EFI_IMG) $(GENESIS_DIR)/iso/boot/efiboot.img
	cp $(GENESIS_APP) $(GENESIS_DIR)/iso/EFI/BOOT/BOOTX64.EFI
	xorriso -as mkisofs -R -J -V EXPOS_GENESIS -e boot/efiboot.img -no-emul-boot -append_partition 2 0xef $(GENESIS_EFI_IMG) -appended_part_as_gpt -o $@ $(GENESIS_DIR)/iso

genesis-iso: $(GENESIS_ISO)

genesis-check: $(GENESIS_ISO)
	OVMF_CODE=$(OVMF_CODE) OVMF_VARS=$(OVMF_VARS) python3 tests/check-genesis.py

# Native firmware image: the validated ELF payload is embedded in a relocatable
# PE32+ UEFI application. No third-party bootloader runs in this path.
$(BUILD)/kernel-uefi.elf: linker.ld $(BUILD)/boot.o kernel-build
	ld --defsym=KERNEL_LOAD_BASE=0x2000000 -T linker.ld -e _uefi_start -o $@ $(BUILD)/boot.o $(RUST_LIB)

$(BUILD)/uefi-loader.obj: boot/uefi/loader.c | $(BUILD)
	clang --target=x86_64-w64-windows-gnu -O2 -Wall -Wextra -Werror -ffreestanding -fno-builtin -fno-stack-protector -mno-red-zone -mno-stack-arg-probe -c $< -o $@

$(BUILD)/uefi-payload.obj: boot/uefi/payload.asm $(BUILD)/kernel-uefi.elf
	nasm -f win64 $< -o $@

$(UEFI_APP): $(BUILD)/uefi-loader.obj $(BUILD)/uefi-payload.obj
	mkdir -p $(dir $@)
	ld -mi386pep --subsystem 10 --entry efi_main --image-base 0 --enable-reloc-section -o $@ $^

$(UEFI_BOOT_IMG): $(UEFI_APP) tools/make-efi-fat.py
	python3 tools/make-efi-fat.py $(UEFI_APP) $@

.PHONY: uefi run-uefi uefi-check bootmode-check startup-check
uefi: $(UEFI_APP)

run-uefi: $(UEFI_APP) $(STATE_IMG)
	cp $(OVMF_VARS) $(BUILD)/OVMF-run-vars.fd
	$(QEMU) -drive if=pflash,format=raw,readonly=on,file=$(OVMF_CODE) -drive if=pflash,format=raw,file=$(BUILD)/OVMF-run-vars.fd -drive file=$(STATE_IMG),format=raw,if=ide,index=0 -drive file=fat:rw:$(UEFI_DIR),format=raw,if=ide,index=1 -serial stdio -no-reboot

startup-check: $(ISO) $(UEFI_BOOT_IMG)
	TMPDIR=/tmp OVMF_CODE=$(OVMF_CODE) OVMF_VARS=$(OVMF_VARS) python3 tests/check-startup.py uefi
	python3 tests/check-startup.py bios
	@echo ">>> EXPOS VISIBLE STARTUP TESTS PASSED <<<"

uefi-check: $(UEFI_BOOT_IMG)
	cp $(OVMF_VARS) $(BUILD)/OVMF-uefi-vars.fd
	truncate -s 0 $(BUILD)/uefi-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/uefi-state.img
	TMPDIR=/tmp OVMF_CODE=$(OVMF_CODE) python3 tests/check-uefi.py uefi
	grep -q 'EXPOS_UEFI_HANDOFF version=1 boot_services=exited' $(BUILD)/uefi-serial.log
	grep -q 'EXPOS_BOOT_OK' $(BUILD)/uefi-serial.log
	grep -q 'EXPOS_LOGIN_OK user=operator' $(BUILD)/uefi-serial.log
	grep -q 'EXPOS_COMMAND_OK shutdown' $(BUILD)/uefi-serial.log
	@echo ">>> EXPOS NATIVE UEFI TEST PASSED <<<"

bootmode-check: $(UEFI_BOOT_IMG)
	cp $(OVMF_VARS) $(BUILD)/OVMF-single-vars.fd
	truncate -s 0 $(BUILD)/single-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/single-state.img
	TMPDIR=/tmp OVMF_CODE=$(OVMF_CODE) python3 tests/check-uefi.py single
	grep -q 'EXPOS_SERVICE_MODE single-user network=disabled desktop=disabled operator-only=true' $(BUILD)/single-serial.log
	grep -q 'EXPOS_SINGLE_USER_DENIED non-operator' $(BUILD)/single-serial.log
	grep -q 'EXPOS_LOGIN_OK user=operator' $(BUILD)/single-serial.log
	! grep -q 'EXPOS_NET_READY' $(BUILD)/single-serial.log
	grep -q 'EXPOS_SINGLE_USER_DENIED desktop' $(BUILD)/single-serial.log
	grep -q 'EXPOS_SINGLE_USER_DENIED ping' $(BUILD)/single-serial.log
	grep -q 'EXPOS_COMMAND_OK shutdown' $(BUILD)/single-serial.log
	@echo ">>> EXPOS SINGLE USER TEST PASSED <<<"

check: $(ISO)
	rm -f $(BUILD)/serial.log
	rm -f $(BUILD)/check-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/check-state.img
	set +e; timeout 30 $(QEMU) -drive file=$(BUILD)/check-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-smoke-input.txt > $(BUILD)/serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_BOOT_OK" $(BUILD)/serial.log
	grep -Eq "EXPOS_BOOT_SCREEN_PRESENTED preset=480p frames=[1-9][0-9]* pageflip=true y_offset=480 visible=true" $(BUILD)/serial.log
	grep -q "EXPOS_BOOT_MODE console" $(BUILD)/serial.log
	grep -q "EXPOS_LOGIN_OK user=operator" $(BUILD)/serial.log
	grep -q "EXPOS_SHELL_READY" $(BUILD)/serial.log
	grep -q "EXPOS_COMMAND_OK help" $(BUILD)/serial.log
	grep -q "EXPOS_USER_CREATED artist" $(BUILD)/serial.log
	grep -q "artist (Power authority)" $(BUILD)/serial.log
	grep -q "EXPOS_PASSWORD_CHANGED artist" $(BUILD)/serial.log
	grep -q "EXPOS_USER_DELETED artist" $(BUILD)/serial.log
	grep -q "KERNEL FEATURE MATRIX" $(BUILD)/serial.log
	grep -Fq ".--------.   .--------." $(BUILD)/serial.log
	grep -q "kern.event.batch=4 (u64, operator-write)" $(BUILD)/serial.log
	grep -q "EXPOS_SYSCTL_CHANGED node=kern.event.batch value=8" $(BUILD)/serial.log
	grep -q "watch added: signal 7" $(BUILD)/serial.log
	grep -q "signal 7 ready with data=42" $(BUILD)/serial.log
	grep -Eq "[[:space:]]7[[:space:]]+signal[[:space:]]+42" $(BUILD)/serial.log
	grep -q "watch added: resource 2" $(BUILD)/serial.log
	grep -q "EXPOS_EXPBUDGET_CHANGED resource=scratch-pages soft=2 hard=4" $(BUILD)/serial.log
	grep -q "expbudget: scratch-pages: resource soft limit reached" $(BUILD)/serial.log
	grep -q "expbudget: event-watches: resource usage is managed by its owning subsystem" $(BUILD)/serial.log
	grep -q "DIESE denied: register an event watch requires Execute capability" $(BUILD)/serial.log
	grep -Eq "[[:space:]]2[[:space:]]+resource[[:space:]]+3" $(BUILD)/serial.log
	grep -q "TSC frequency: .* Hz" $(BUILD)/serial.log
	grep -q "clock source:" $(BUILD)/serial.log
	grep -q "EXPOS DIAGNOSTIC REPORT" $(BUILD)/serial.log
	grep -q "DISPLAY DIAGNOSTICS" $(BUILD)/serial.log
	grep -q "requested: 480p 640x480x32" $(BUILD)/serial.log
	grep -q "STATE DIAGNOSTICS" $(BUILD)/serial.log
	grep -q "display: preset-id=0 refresh=60Hz vsync=on" $(BUILD)/serial.log
	grep -q "EXPOS_COMMAND_OK displaydiag" $(BUILD)/serial.log
	grep -q "EXPOS_COMMAND_OK stateinfo" $(BUILD)/serial.log
	grep -q "EXPOS_COMMAND_OK diag" $(BUILD)/serial.log
	grep -q "EXPOS_SAFE_VIDEO_APPLIED persisted=true" $(BUILD)/serial.log
	grep -q "EXPOS_COMMAND_OK safevideo" $(BUILD)/serial.log
	awk '/EXPOS_SAFE_VIDEO_APPLIED persisted=true/{applied=1; next} applied && /display: preset-id=0 refresh=60Hz vsync=on/{verified=1} END{exit !verified}' $(BUILD)/serial.log
	grep -q "Ayo.*package" $(BUILD)/serial.log
	grep -q "ExpDisplay.*service" $(BUILD)/serial.log
	grep -q "FormABI.*interface" $(BUILD)/serial.log
	grep -q "Ayo --configured-by--> Root" $(BUILD)/serial.log
	grep -q "DemoBrowser --depends-on--> Ayo" $(BUILD)/serial.log
	grep -q "DIESE exact resolution" $(BUILD)/serial.log
	grep -q "Removed 'DemoBrowser' --depends-on--> 'Ayo'" $(BUILD)/serial.log
	grep -q "Reclaimed 'Scratch'" $(BUILD)/serial.log
	grep -q "Created and bound 'DemoBrowser'" $(BUILD)/serial.log
	grep -q "Granted Handle" $(BUILD)/serial.log
	grep -q "EXPOS_FORM_SCHEDULED context=.*fin=.*dimension=" $(BUILD)/serial.log
	grep -q "Scheduled 'DemoBrowser' as context" $(BUILD)/serial.log
	grep -q "hello from scheduled Form" $(BUILD)/serial.log
	grep -q "EXPOS_FORM_EXITED context=.*result=0" $(BUILD)/serial.log
	grep -q "Executable Form 'Hello' exited with result 0" $(BUILD)/serial.log
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
	grep -Eq "EXPOS_BOOT_SCREEN_PRESENTED preset=480p frames=[1-9][0-9]* pageflip=true y_offset=480 visible=true" $(BUILD)/display-serial.log
	grep -Eq "EXPOS_LOGIN_SCREEN_PRESENTED preset=480p frames=[1-9][0-9]* pageflip=true y_offset=480 visible=true" $(BUILD)/display-serial.log
	grep -q "framebuffer: 640x480 XRGB8888 scanout available=true" $(BUILD)/display-serial.log
	grep -q "EXPOS_BOOT_MODE graphical" $(BUILD)/display-serial.log
	grep -q "EXPOS_DISPLAY_MODE width=640 height=480 bpp=32 stride=2560 bytes=1228800 preset=480p" $(BUILD)/display-serial.log
	grep -q "EXPOS_DISPLAY_MODE .*pageflip=true" $(BUILD)/display-serial.log
	grep -q "EXPOS_PRESENTATION_READY rate=60 Hz vsync=true pageflip=true" $(BUILD)/display-serial.log
	awk '/EXPOS_RENDER_POLICY/{first=1; if ($$0 ~ /mode=Efficient damage=true shadows=false wallpaper_effects=false/) safe=1; exit} END{exit !(first && safe)}' $(BUILD)/display-serial.log
	grep -q "EXPOS_DISPLAY_READY surfaces=11 commit=11" $(BUILD)/display-serial.log
	grep -q "EXPOS_DESKTOP_EMPTY open_apps=0 pinned_apps=0" $(BUILD)/display-serial.log
	grep -q "EXPOS_MOUSE_READY enabled=true" $(BUILD)/display-serial.log
	grep -q "EXPOS_APP_OPENED SETTINGS" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=theme value=Graphite" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=resolution value=720p" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=resolution value=1080p" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=resolution value=480p" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=refresh-rate value=75 Hz" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=refresh-rate value=120 Hz" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=refresh-rate value=144 Hz" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=vsync value=off" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=vsync value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=window-shadows value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=wallpaper-effects value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=presentation-policy value=Responsive" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=font-face value=Rounded" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=font-weight value=Bold" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=window-radius value=2 px" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=window-border-width value=1 px" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=titlebar-size value=Small" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=window-opacity value=96%" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=offscreen-allowance value=8 px" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=window-snap value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=snap-distance value=4 px" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=focus-policy value=Sloppy" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-placement value=Top" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-size value=32 px" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-alignment value=Center" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-autohide value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-translucent value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-labels value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=clock-seconds value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_DISPLAY_MODE width=640 height=480 bpp=32 stride=2560 bytes=1228800 preset=480p" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar value=off" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=network value=off" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_CHANGED key=network value=on" $(BUILD)/display-serial.log
	grep -q "EXPOS_SETTING_DENIED key=bluetooth error=BusUnsupported" $(BUILD)/display-serial.log
	grep -q "EXPOS_APP_OPENED TERMINAL" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=help" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=neofetch" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=status" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=storage" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=theme" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=ps" $(BUILD)/display-serial.log
	grep -q "EXPOS_TERMINAL_COMMAND name=windowreset" $(BUILD)/display-serial.log
	grep -q "EXPOS_WINDOW_LAYOUT_RESET count=8" $(BUILD)/display-serial.log
	grep -q "EXPOS_APP_CLOSED TERMINAL" $(BUILD)/display-serial.log
	grep -q "EXPOS_APP_REOPENED TERMINAL" $(BUILD)/display-serial.log
	grep -Eq "EXPOS_PRESENTATION_STATS frames=[1-9][0-9]* missed=[0-9]+ idle=[0-9]+ vblank_timeouts=0 responsive_commits=[1-9][0-9]*" $(BUILD)/display-serial.log
	grep -Eq "EXPOS_RENDER_STATS full=[1-9][0-9]* damaged=[1-9][0-9]* callbacks=[1-9][0-9]* surface_frames=[1-9][0-9]* pointer_merged=[0-9]+ submitted_regions=[1-9][0-9]* copied_regions=[1-9][0-9]* copied_pixels=[1-9][0-9]* collapses=[0-9]+" $(BUILD)/display-serial.log
	grep -Eq "damage: submitted-regions=[1-9][0-9]* submitted-pixels=[1-9][0-9]* copied-regions=[1-9][0-9]* copied-pixels=[1-9][0-9]* collapses=[0-9]+" $(BUILD)/display-serial.log
	grep -q "EXPOS_DISPLAY_CLOSED" $(BUILD)/display-serial.log
	grep -q "EXPOS_COMMAND_OK desktop" $(BUILD)/display-serial.log
	grep -q "EXPOS_PRESENTATION_READY rate=144 Hz vsync=true pageflip=true" $(BUILD)/display-serial.log
	grep -q "EXPOS_RENDER_POLICY mode=Responsive damage=true shadows=true wallpaper_effects=true" $(BUILD)/display-serial.log
	grep -q "ExpOS Form ABI v1" $(BUILD)/display-serial.log
	@echo ">>> EXPOS DISPLAY TEST PASSED <<<"

session-check: $(ISO)
	rm -f $(BUILD)/guest-serial.log
	rm -f $(BUILD)/guest-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/guest-state.img
	set +e; timeout 30 $(QEMU) -drive file=$(BUILD)/guest-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-guest-input.txt > $(BUILD)/guest-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_LOGIN_OK user=guest" $(BUILD)/guest-serial.log
	grep -q "guest (Guest authority)" $(BUILD)/guest-serial.log
	grep -q "DIESE denied: change a kernel tunable requires Operator capability" $(BUILD)/guest-serial.log
	grep -q "DIESE denied: register an event watch requires Execute capability" $(BUILD)/guest-serial.log
	grep -q "DIESE denied: change a resource ceiling requires Operator capability" $(BUILD)/guest-serial.log
	grep -q "DIESE denied 'mkform' for Guest authority" $(BUILD)/guest-serial.log
	grep -q "DIESE denied 'safevideo' for Guest authority" $(BUILD)/guest-serial.log
	grep -q "DIESE denied 'displayreset' for Guest authority" $(BUILD)/guest-serial.log
	grep -q "EXPOS_COMMAND_DENIED safevideo" $(BUILD)/guest-serial.log
	grep -q "EXPOS_COMMAND_DENIED displayreset" $(BUILD)/guest-serial.log
	! grep -q "Created and bound 'Forbidden'" $(BUILD)/guest-serial.log
	grep -q "EXPOS_COMMAND_OK shutdown" $(BUILD)/guest-serial.log
	@echo ">>> EXPOS SESSION AUTHORITY TEST PASSED <<<"

network-check: $(ISO)
	rm -f $(BUILD)/network-serial.log
	rm -f $(BUILD)/network-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/network-state.img
	python3 -m http.server 18080 --bind 127.0.0.1 --directory tests > $(BUILD)/http-fixture.log 2>&1 & fixture_pid=$$!; \
	trap 'kill $$fixture_pid 2>/dev/null || true' EXIT; \
	set +e; timeout 35 $(QEMU) -drive file=$(BUILD)/network-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-network-input.txt > $(BUILD)/network-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_NET_READY driver=rtl8139" $(BUILD)/network-serial.log
	grep -q "EXPOS_DHCP_BOUND address=10.0.2.15 gateway=10.0.2.2 dns=10.0.2.3" $(BUILD)/network-serial.log
	grep -q "ether0  up" $(BUILD)/network-serial.log
	grep -q "ipv4=dhcp dns=10.0.2.3" $(BUILD)/network-serial.log
	grep -q "EXPOS_HTTP_OK status=200 bytes=38 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "ExpOS native TCP and HTTP are online." $(BUILD)/network-serial.log
	grep -q "EXPOS_BROWSER_ENGINE nodes=4 css_rules=5 scripts=1 executed=1 rejected=0 handlers=1" $(BUILD)/network-serial.log
	grep -q "EXPOS_BROWSER_HTTP_OK status=200 bytes=786 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "EXPOS_BROWSER_REDIRECT status=301 hop=1" $(BUILD)/network-serial.log
	grep -q "EXPOS_BROWSER_HTTP_OK status=200 bytes=134 peer=10.0.2.2" $(BUILD)/network-serial.log
	grep -q "EXPOS_PING_REPLY address=10.0.2.2 sequence=1" $(BUILD)/network-serial.log
	grep -q "EXPOS_PING_SUMMARY sent=160 received=160" $(BUILD)/network-serial.log
	! grep -q "EXPOS_PING_ERROR" $(BUILD)/network-serial.log
	! grep -q "EXPOS_HTTP_ERROR" $(BUILD)/network-serial.log
	! grep -q "EXPOS_BROWSER_HTTP_ERROR" $(BUILD)/network-serial.log
	grep -q "rtl8139-poll up" $(BUILD)/network-serial.log
	@echo ">>> EXPOS NATIVE NETWORK TEST PASSED <<<"

internet-check: $(ISO)
	rm -f $(BUILD)/internet-serial.log
	rm -f $(BUILD)/internet-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/internet-state.img
	set +e; timeout 35 $(QEMU) -drive file=$(BUILD)/internet-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-internet-input.txt > $(BUILD)/internet-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_DNS_OK host=example.com address=" $(BUILD)/internet-serial.log
	grep -q "EXPOS_HTTP_OK status=" $(BUILD)/internet-serial.log
	! grep -q "EXPOS_DNS_ERROR" $(BUILD)/internet-serial.log
	! grep -q "EXPOS_HTTP_ERROR" $(BUILD)/internet-serial.log
	@echo ">>> EXPOS LIVE INTERNET TEST PASSED <<<"

https-check: $(ISO)
	rm -f $(BUILD)/https-serial.log
	rm -f $(BUILD)/https-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/https-state.img
	set +e; timeout 60 $(QEMU) -drive file=$(BUILD)/https-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-https-input.txt > $(BUILD)/https-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_TLS_VERIFIED host=www.youtube.com version=1.3" $(BUILD)/https-serial.log
	grep -q "EXPOS_HTTP_OK status=" $(BUILD)/https-serial.log
	grep -q "EXPOS_BROWSER_HTTP_OK status=200" $(BUILD)/https-serial.log
	! grep -q "EXPOS_HTTP_ERROR" $(BUILD)/https-serial.log
	! grep -q "EXPOS_BROWSER_HTTP_ERROR" $(BUILD)/https-serial.log
	@echo ">>> EXPOS VERIFIED HTTPS TEST PASSED <<<"

search-check: $(ISO)
	rm -f $(BUILD)/search-serial.log
	rm -f $(BUILD)/search-state.img
	truncate -s $(STATE_SIZE) $(BUILD)/search-state.img
	set +e; timeout 60 $(QEMU) -drive file=$(BUILD)/search-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-search-input.txt > $(BUILD)/search-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_BROWSER_SEARCH query_bytes=23 .*provider=duckduckgo-html" $(BUILD)/search-serial.log
	grep -q "EXPOS_TLS_VERIFIED host=duckduckgo.com version=1.3" $(BUILD)/search-serial.log
	grep -Eq 'EXPOS_SEARCH_RESULTS count=[1-8][[:space:]]*$$' $(BUILD)/search-serial.log
	grep -q "EXPOS_BROWSER_HTTP_OK status=200" $(BUILD)/search-serial.log
	! grep -q "EXPOS_BROWSER_HTTP_ERROR" $(BUILD)/search-serial.log
	@echo ">>> EXPOS DUCKDUCKGO HTML SEARCH TEST PASSED <<<"

persistence-check: $(ISO)
	rm -f $(BUILD)/persistence-state.img $(BUILD)/persistence-write.log $(BUILD)/persistence-read.log
	truncate -s $(STATE_SIZE) $(BUILD)/persistence-state.img
	set +e; timeout 40 $(QEMU) -drive file=$(BUILD)/persistence-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-persistence-write-input.txt > $(BUILD)/persistence-write.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	set +e; timeout 40 $(QEMU) -drive file=$(BUILD)/persistence-state.img,format=raw,if=ide,index=0 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-persistence-read-input.txt > $(BUILD)/persistence-read.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q "EXPOS_USER_CREATED keeper" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=theme value=Graphite" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=resolution value=480p" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=refresh-rate value=144 Hz" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=vsync value=off" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=window-shadows value=on" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=wallpaper-effects value=on" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=presentation-policy value=Responsive" $(BUILD)/persistence-write.log
	grep -q "EXPOS_SETTING_CHANGED key=taskbar-placement value=Top" $(BUILD)/persistence-write.log
	grep -q "EXPOS_STATE_COMMIT generation=" $(BUILD)/persistence-write.log
	grep -q "EXPOS_EXPFS_COMMIT generation=" $(BUILD)/persistence-write.log
	grep -q "Created and bound 'MyNotes'" $(BUILD)/persistence-write.log
	grep -q "Granted Handle .* execute on MyNotes" $(BUILD)/persistence-write.log
	grep -q "Created ExpFS checkpoint 1" $(BUILD)/persistence-write.log
	! grep -q "violetmemory" $(BUILD)/persistence-write.log
	! LC_ALL=C grep -a -q "violetmemory" $(BUILD)/persistence-state.img
	grep -q "EXPOS_STATE_READY generation=.*slot=expfs loaded=true" $(BUILD)/persistence-read.log
	grep -q "EXPOS_ACCOUNTS_READY source=disk persisted=true" $(BUILD)/persistence-read.log
	grep -q "EXPOS_EXPFS_FORMS_RESTORED" $(BUILD)/persistence-read.log
	grep -q "^hello" $(BUILD)/persistence-read.log
	grep -q "Scheduled 'MyNotes' as context" $(BUILD)/persistence-read.log
	grep -Eq "^1[[:space:]]+[0-9]+[[:space:]]+0" $(BUILD)/persistence-read.log
	grep -q "EXPOS_LOGIN_OK user=keeper" $(BUILD)/persistence-read.log
	grep -q "keeper (Power authority)" $(BUILD)/persistence-read.log
	grep -q "DIESE denied 'safevideo' for Power authority" $(BUILD)/persistence-read.log
	grep -q "DIESE denied 'displayreset' for Power authority" $(BUILD)/persistence-read.log
	grep -q "EXPOS_COMMAND_DENIED safevideo" $(BUILD)/persistence-read.log
	grep -q "EXPOS_COMMAND_DENIED displayreset" $(BUILD)/persistence-read.log
	! grep -q "EXPOS_SAFE_VIDEO_APPLIED" $(BUILD)/persistence-read.log
	grep -q "EXPOS_DISPLAY_MODE width=640 height=480 bpp=32 stride=2560 bytes=1228800 preset=480p" $(BUILD)/persistence-read.log
	grep -q "EXPOS_DESKTOP_PREFS theme=Graphite" $(BUILD)/persistence-read.log
	grep -q "EXPOS_CUSTOMIZATION font=System weight=Regular .*taskbar=Top size=28 px align=Start" $(BUILD)/persistence-read.log
	grep -q "EXPOS_PRESENTATION_READY rate=144 Hz vsync=false pageflip=true" $(BUILD)/persistence-read.log
	grep -q "EXPOS_RENDER_POLICY mode=Responsive damage=true shadows=true wallpaper_effects=true" $(BUILD)/persistence-read.log
	grep -Eq "EXPOS_RENDER_STATS full=[1-9][0-9]* damaged=[0-9]+ callbacks=[1-9][0-9]* surface_frames=[1-9][0-9]* pointer_merged=[0-9]+ submitted_regions=[1-9][0-9]* copied_regions=[1-9][0-9]* copied_pixels=[1-9][0-9]* collapses=[0-9]+" $(BUILD)/persistence-read.log
	! grep -q "violetmemory" $(BUILD)/persistence-read.log
	grep -q "EXPOS_COMMAND_OK shutdown" $(BUILD)/persistence-read.log
	@echo ">>> EXPOS PERSISTENCE TEST PASSED <<<"

run: run-uefi

.PHONY: run-bios
run-bios: $(ISO) $(STATE_IMG)
	$(QEMU) -drive file=$(STATE_IMG),format=raw,if=ide,index=0 -boot once=d -cdrom $(ISO) -serial stdio -no-reboot

debug: $(ISO) $(STATE_IMG)
	$(QEMU) -drive file=$(STATE_IMG),format=raw,if=ide,index=0 -boot once=d -cdrom $(ISO) -display none -serial stdio -no-reboot -s -S

ayo:
	$(MAKE) -C ayo test build

sdk: rust-sdk go-sdk c-sdk python-sdk sdk-cli-check

rust-sdk:
	cargo test -p expos-sdk

go-sdk:
	GOCACHE=/tmp/expos-go-sdk-cache go -C sdk/go test ./...

c-sdk:
	$(CC) -std=c11 -Wall -Wextra -Werror -pedantic -Isdk/c sdk/c/tests/abi_test.c -o /tmp/expos-c-sdk-test
	/tmp/expos-c-sdk-test

sdk-cli-check:
	python3 -m unittest discover -s sdk/cli/tests -v

python-sdk:
	PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/python/tests -v

python-runtime-check:
	python3 ports/python/test_runtime.py

budget-check: $(ISO)
	python3 tests/check-budget.py

python-check: $(ISO)
	set +e; timeout 30 $(QEMU) -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom $(ISO) -display none -serial stdio -no-reboot < tests/qemu-python-input.txt > $(BUILD)/python-serial.log 2>&1; qemu_status=$$?; test $$qemu_status -eq 33
	grep -q '^30' $(BUILD)/python-serial.log
	grep -q 'hello from a Form' $(BUILD)/python-serial.log
	test "$$(grep -c 'ExpPython: execution budget exhausted; script stopped.' $(BUILD)/python-serial.log)" -eq 3
	grep -q 'MemoryError: memory allocation failed' $(BUILD)/python-serial.log
	grep -q '^recovered' $(BUILD)/python-serial.log
	grep -q 'DIESE denied: run Python requires Execute capability' $(BUILD)/python-serial.log
	! grep -q '^forbidden' $(BUILD)/python-serial.log
	grep -q 'EXPOS_COMMAND_OK shutdown' $(BUILD)/python-serial.log
	@echo ">>> EXPOS NATIVE PYTHON TEST PASSED <<<"

legacy-alpha-check:
	$(MAKE) -C legacy/alpha32 clean all

run-alpha:
	$(MAKE) -C legacy/alpha32 run

clean:
	cargo clean
	$(MAKE) -C ayo clean
	rm -rf $(BUILD) iso/boot/kernel.elf
