#!/usr/bin/env python3
"""Wait for kernel readiness before feeding input, so firmware cannot eat it."""
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import sys
import time

root = Path(__file__).resolve().parents[1]
single = len(sys.argv) > 1 and sys.argv[1] == "single"
name = "single" if single else "uefi"
code = os.environ.get("OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd")
boot_image = root / f"build/{name}-test-boot.img"
shutil.copyfile(root / "build/uefi-boot.img", boot_image)
command = ["qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
           "-vga", "std", "-global", "VGA.vgamem_mb=16", "-netdev", "user,id=net0",
           "-device", "rtl8139,netdev=net0", "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
           "-drive", f"if=pflash,format=raw,file=build/OVMF-{name}-vars.fd",
           "-drive", f"file=build/{name}-state.img,format=raw,if=ide,index=0",
           "-drive", f"file={boot_image},format=raw,if=ide,index=1",
           "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-display", "none",
           "-serial", "stdio", "-no-reboot"]
process = subprocess.Popen(command, cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output = bytearray()
sent = False
deadline = time.monotonic() + 45
try:
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.2):
            chunk = os.read(key.fd, 16384)
            if chunk:
                output.extend(chunk)
                if not sent and b"EXPOS_BOOT_MODE_READY" in output:
                    process.stdin.write((root / f"tests/qemu-{name}-input.txt").read_bytes())
                    process.stdin.flush()
                    sent = True
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError("UEFI boot test timed out")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    (root / f"build/{name}-serial.log").write_bytes(output)
assert process.returncode == 33, f"boot failed ({process.returncode}); see build/{name}-serial.log"
print(f"ExpOS {name} firmware run completed")
