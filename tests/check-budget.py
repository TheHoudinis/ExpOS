#!/usr/bin/env python3
"""Exercise owned resource accounting through the real booted shell."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
state = root / "build/budget-state.img"
with state.open("wb") as image:
    image.truncate(4 * 1024 * 1024)
result = subprocess.run(
    ["qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
     "-vga", "std", "-global", "VGA.vgamem_mb=16", "-netdev", "user,id=net0",
     "-device", "rtl8139,netdev=net0", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
     "-drive", f"file={state},format=raw,if=ide,index=0",
     "-cdrom", str(root / "build/expos.iso"), "-display", "none", "-serial", "stdio", "-no-reboot"],
    input=(root / "tests/qemu-budget-input.txt").read_bytes(),
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30,
)
output = result.stdout.decode(errors="replace").replace("\r", "")
(root / "build/budget-serial.log").write_text(output)
assert result.returncode == 33, f"boot failed: {result.returncode}; see build/budget-serial.log"
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
