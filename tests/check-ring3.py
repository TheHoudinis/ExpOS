#!/usr/bin/env python3
"""Prove CPL3 execution, distinct CR3 roots and timer budget enforcement."""

from pathlib import Path
import os
import re
import selectors
import subprocess
import time


root = Path(__file__).resolve().parents[1]
state = root / "build/ring3-state.img"
with state.open("wb") as image:
    image.truncate(4 * 1024 * 1024)

command = [
    "qemu-system-x86_64",
    "-machine", "pc",
    "-cpu", "max",
    "-m", "256M",
    "-vga", "std",
    "-global", "VGA.vgamem_mb=16",
    "-netdev", "user,id=net0",
    "-device", "rtl8139,netdev=net0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-drive", f"file={state},format=raw,if=ide,index=0",
    "-cdrom", str(root / "build/expos.iso"),
    "-display", "none",
    "-serial", "stdio",
    "-no-reboot",
]
process = subprocess.Popen(
    command,
    cwd=root,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
captured = bytearray()
sent = False
timed_out = False
deadline = time.monotonic() + 45
try:
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.2):
            chunk = os.read(key.fd, 16384)
            if chunk:
                captured.extend(chunk)
                if not sent and b"EXPOS_BOOT_MODE_READY" in captured:
                    process.stdin.write(
                        (root / "tests/qemu-ring3-input.txt").read_bytes()
                    )
                    process.stdin.flush()
                    sent = True
        if process.poll() is not None:
            captured.extend(process.stdout.read())
            break
    else:
        timed_out = True
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()

output = captured.decode(errors="replace").replace("\r", "")
(root / "build/ring3-serial.log").write_text(output)
assert not timed_out, "Ring 3 boot test timed out; see build/ring3-serial.log"
assert process.returncode == 33, (
    f"ring3 boot failed: {process.returncode}; see build/ring3-serial.log"
)

for expected in (
    "EXPOS_FORM_PLATFORM_READY cpl=3 pit_hz=1000 abi_vector=0x80",
    "hello from isolated Ring 3",
    "call=1",
    "call=8",
    "EXPOS_FORM_FAULT fin=",
    "vector=14",
    "Executable Form 'EscapeProbe' faulted at vector 14.",
    "EXPOS_FORM_PREEMPT",
    "EXPOS_FORM_BUDGET_EXHAUSTED",
    "ExpBudget stopped 'BudgetLoop' after 256 hardware timer ticks.",
    "budget-blocked",
    "EXPOS_COMMAND_OK shutdown",
):
    assert expected in output, f"missing {expected!r}; see build/ring3-serial.log"

roots = re.findall(r"EXPOS_RING3_ENTER .*? cr3=(0x[0-9a-f]+)", output)
assert len(roots) == 3, f"expected three Ring 3 launches, got {roots!r}"
assert len(set(roots)) == 3, f"Forms shared a CR3 root: {roots!r}"
assert "[KERNEL PANIC]" not in output
print("EXPOS RING 3 / VM / PREEMPTION TEST PASSED")
