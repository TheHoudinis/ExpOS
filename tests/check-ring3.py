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
    "-smp", "4",
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
    "EXPOS_SMP_READY discovered=4 online=4",
    "EXPOS_SMP_FORM_DISPATCH",
    "EXPOS_SMP_FORM_COMPLETE",
    "EXPOS_SMP_PARALLEL_START",
    "parallel left",
    "parallel right",
    "EXPOS_SMP_PARALLEL_COMPLETE",
    "ExpBudget stopped 'BudgetLoop' after 256 hardware timer ticks.",
    "budget-blocked",
    "EXPOS_COMMAND_OK shutdown",
):
    assert expected in output, f"missing {expected!r}; see build/ring3-serial.log"

roots = re.findall(
    r"EXPOS_RING3_(?:ENTER|PARALLEL_ADMIT) .*? cr3=(0x[0-9a-f]+)", output
)
assert len(roots) == 5, f"expected five Ring 3 launches, got {roots!r}"
assert len(set(roots)) == 5, f"Forms shared a CR3 root: {roots!r}"

parallel_start = output.index("EXPOS_SMP_PARALLEL_START")
parallel_end = output.index("EXPOS_SMP_PARALLEL_COMPLETE", parallel_start)
parallel_log = output[parallel_start:parallel_end]
dispatches = re.findall(
    r"EXPOS_SMP_FORM_DISPATCH fin=([0-9A-F-]+) cpu_slot=(\d+)", parallel_log
)
completions = re.findall(
    r"EXPOS_SMP_FORM_COMPLETE fin=([0-9A-F-]+) cpu_slot=(\d+)", parallel_log
)
assert len(dispatches) == 2, f"expected two parallel dispatches, got {dispatches!r}"
assert len(completions) == 2, f"expected two parallel completions, got {completions!r}"
assert len({fin for fin, _ in dispatches}) == 2, "same Form dispatched twice"
assert len({slot for _, slot in dispatches}) == 2, "parallel Forms shared one CPU slot"
first_completion = parallel_log.index("EXPOS_SMP_FORM_COMPLETE")
second_dispatch = parallel_log.rfind("EXPOS_SMP_FORM_DISPATCH")
assert second_dispatch < first_completion, "a Form completed before its peer was dispatched"
assert "[KERNEL PANIC]" not in output
print("EXPOS RING 3 / VM / PREEMPTION TEST PASSED")
