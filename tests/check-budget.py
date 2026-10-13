#!/usr/bin/env python3
"""Exercise owned resource accounting through the real booted shell."""
from pathlib import Path
import subprocess
import os
import re
import selectors
import time

root = Path(__file__).resolve().parents[1]
state = root / "build/budget-state.img"
with state.open("wb") as image:
    image.truncate(4 * 1024 * 1024)
process = subprocess.Popen(
    ["qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
     "-vga", "std", "-global", "VGA.vgamem_mb=16", "-netdev", "user,id=net0",
     "-device", "rtl8139,netdev=net0", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
     "-drive", f"file={state},format=raw,if=ide,index=0",
     "-cdrom", str(root / "build/expos.iso"), "-display", "none", "-serial", "stdio", "-no-reboot"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
)
lines = (root / "tests/qemu-budget-input.txt").read_text().splitlines()
output_bytes = bytearray()
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
phase = 0
deadline = time.monotonic() + 45
try:
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 65536)
            output_bytes.extend(chunk)
        text = output_bytes.decode(errors="replace").replace("\r", "")
        if phase == 0 and "EXPOS_BOOT_MODE_READY" in text:
            process.stdin.write(("\n".join(lines[:3]) + "\n").encode())
            process.stdin.flush()
            phase = 1
        elif phase == 1 and "EXPOS_SHELL_READY" in text:
            process.stdin.write(b"expbudget\n")
            process.stdin.flush()
            phase = 2
        elif phase == 2 and "EXPOS_COMMAND_OK expbudget" in text:
            match = re.search(r"^form-bytes\s+(\d+)\s+", text, re.MULTILINE)
            assert match, "could not read resident Form bytes"
            # Keep exactly four bytes available, regardless of changes to the
            # built-in Forms. A stale absolute limit used to fail setup silently.
            soft = int(match[1]) + 4
            assert soft <= 6144, "built-in Forms exceed this fixture's hard limit"
            commands = "\n".join(lines[3:]).replace("@SOFT_LIMIT@", str(soft)) + "\n"
            process.stdin.write(commands.encode())
            process.stdin.flush()
            phase = 3
        if process.poll() is not None:
            output_bytes.extend(process.stdout.read())
            break
    else:
        raise TimeoutError("budget test timed out")
finally:
    if process.poll() is None:
        process.kill()
    process.wait()
    selector.close()
    output = output_bytes.decode(errors="replace").replace("\r", "")
    (root / "build/budget-serial.log").write_text(output)
assert process.returncode == 33, f"boot failed: {process.returncode}; see build/budget-serial.log"
assert "EXPOS_EXPBUDGET_CHANGED resource=form-bytes" in output, "budget setup failed"
for expected in (
    "Form content denied: resource soft limit reached; nothing changed.",
    "\n1234\n",  # Content survives the rejected append.
    "form-bytes: resource usage is managed by its owning subsystem",
    "EXPOS_FORM_BUDGET_DENIED command=mkform",
    "EXPOS_COMMAND_OK shutdown",
):
    assert expected in output, f"missing {expected!r}; see build/budget-serial.log"
assert "Created and bound 'MustNotExist'" not in output
print("EXPOS RESOURCE ENFORCEMENT TEST PASSED")
