SHELL := /bin/bash

CC = gcc
CFLAGS = -m32 -ffreestanding -nostdinc -nostdlib -fno-pie -fno-stack-protector -mno-sse -mno-sse2 -Wall -Wextra -I. -g
LDFLAGS = -T link.ld -m elf_i386 -nostdlib

OBJECTS = kernel_entry.o interrupts.o paging.o process.o syscall.o sync.o log.o driver.o pipe.o vfs.o elf.o hexafs_disk.o hexafs.o kobserve.o boot_policy.o intent.o replay.o hex.o net.o fb.o hexa.o

all: os.img storage.img

boot.bin: boot_entry.asm kernel.bin
	nasm -f bin -o boot.bin boot_entry.asm
	ksect=$$(( $$(stat -c '%s' kernel.bin) / 512 + 1 )); \
	[ $$ksect -gt 510 ] && ksect=510; \
	printf '%b' "\\x$$(printf '%02x' $$((ksect & 0xFF)))" | dd of=boot.bin bs=1 seek=27 count=1 conv=notrunc 2>/dev/null; \
	printf '%b' "\\x$$(printf '%02x' $$((ksect >> 8 & 0xFF)))" | dd of=boot.bin bs=1 seek=28 count=1 conv=notrunc 2>/dev/null; \
	echo "[*] Kernel sectors: $$ksect"

kernel_entry.o: kernel_entry.asm
	nasm -f elf32 -o $@ $<

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

kernel.elf: $(OBJECTS) link.ld
	ld $(LDFLAGS) -o $@ $(OBJECTS) -Map=link.map

kernel.bin: kernel.elf
	objcopy -O binary $< $@

storage.img:
	@if [ ! -f storage.img ]; then dd if=/dev/zero bs=512 count=4096 of=storage.img 2>/dev/null && echo "[✓] Created storage.img"; else echo "[*] storage.img exists, keeping data."; fi

fresh:
	dd if=/dev/zero bs=512 count=4096 of=storage.img 2>/dev/null
	@echo "[✓] Fresh storage.img created (data wiped)"

os.img: boot.bin kernel.bin
	cat boot.bin kernel.bin > os.img
	@truncate -s 1474560 os.img
	@echo "[✓] OS image: $@"

run: os.img storage.img
	@echo "[*] Booting HEXA OS in QEMU..."
	@qemu-system-i386 -fda os.img -hda storage.img -boot order=a -nographic -d cpu_reset -no-reboot -netdev user,id=u1 -device rtl8139,netdev=u1

debug: kernel.elf os.img storage.img
	@echo "[*] Starting QEMU with GDB stub on port 1234..."
	@qemu-system-i386 -fda os.img -hda storage.img -boot order=a -nographic -S -s -netdev user,id=u1 -device rtl8139,netdev=u1 &
	@gdb -ex "target remote localhost:1234" -ex "symbol-file kernel.elf" -ex "b kernel_main" -ex "c"

clean:
	rm -f *.o *.elf *.bin os.img
	@echo "[✓] Cleaned (storage.img preserved)."

.PHONY: all run debug clean fresh
