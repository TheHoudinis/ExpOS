#!/usr/bin/env python3
"""Require one-shot PIT idle in the shell while preserving Ring 3 preemption."""

import os
from pathlib import Path
import selectors
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
disk = BUILD / "tickless-state.img"
log = BUILD / "tickless-serial.log"
disk.write_bytes(b"")
with disk.open("r+b") as stream:
    stream.truncate(8 * 1024 * 1024)

command = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4",
    "-m", "256M", "-vga", "std", "-display", "none",
    "-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0",
    "-drive", f"file={disk},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-boot", "once=d", "-cdrom", str(BUILD / "expos.iso"),
    "-serial", "stdio", "-no-reboot",
]
process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output, sent, deadline = bytearray(), False, time.monotonic() + 90
marker = b"EXPOS_TICKLESS_IDLE mode=pit-one-shot maximum_us=50000"
try:
    process.stdin.write(b"2222\noperator\nexpos\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 16384)
            if chunk:
                output.extend(chunk)
        if not sent and marker in output:
            process.stdin.write(b"shutdown\n")
            process.stdin.flush()
            sent = True
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError("tickless guest timed out")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    log.write_bytes(output)

text = output.decode(errors="replace")
assert sent and process.returncode == 33, f"tickless guest failed; see {log}"
assert "ring3_preemption_hz=1000" in text, "Ring 3 preemption contract changed"
assert "EXPOS_COMMAND_OK shutdown" in text, "guest did not shut down cleanly"
