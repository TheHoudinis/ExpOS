#!/usr/bin/env python3
"""Prove e1000 or transitional VirtIO-net DHCP, TX/RX and IRQ operation."""

from pathlib import Path
import os
import selectors
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
backend = sys.argv[1]
devices = {
    "e1000": "e1000,netdev=net0",
    "e1000e": "e1000e,netdev=net0",
    "virtio-net": "virtio-net-pci,netdev=net0,disable-modern=on",
}
if backend not in devices:
    raise SystemExit(f"unsupported backend {backend}")
disk = BUILD / f"{backend}-network-state.img"
disk.write_bytes(b"")
with disk.open("r+b") as stream:
    stream.truncate(8 * 1024 * 1024)
log = BUILD / f"{backend}-network.log"
command = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4",
    "-m", "256M", "-vga", "std", "-display", "none",
    "-netdev", "user,id=net0", "-device", devices[backend],
    "-drive", f"file={disk},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-boot", "once=d", "-cdrom", str(BUILD / "expos.iso"),
    "-serial", "stdio", "-no-reboot",
]
process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output, followup_sent, deadline = bytearray(), False, time.monotonic() + 120
try:
    process.stdin.write(b"2222\noperator\nexpos\nifconfig\nping 10.0.2.2 2\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 16384)
            if chunk:
                output.extend(chunk)
        if not followup_sent and b"EXPOS_PING_SUMMARY sent=2 received=2" in output:
            # Leave one real idle window so the completion interrupt can run
            # before netstat samples the event counter.
            time.sleep(0.2)
            process.stdin.write(b"netstat\nshutdown\n")
            process.stdin.flush()
            followup_sent = True
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError(f"{backend} guest timed out")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    log.write_bytes(output)
text = output.decode(errors="replace")
assert process.returncode == 33, f"{backend} guest failed; see {log}"
intel = backend in ("e1000", "e1000e")
marker = "EXPOS_E1000_READY" if intel else "EXPOS_VIRTIO_NET_READY"
assert marker in text, f"{backend} did not initialize; see {log}"
driver = "e1000" if intel else backend
assert f"EXPOS_NET_READY driver={driver}" in text, f"wrong selected backend; see {log}"
assert (
    "EXPOS_DHCP_BOUND address=10.0.2.15" in text
    or "EXPOS_DHCP_FALLBACK" in text
), f"network configuration failed; see {log}"
assert "EXPOS_PING_REPLY address=10.0.2.2 sequence=1" in text, f"RX/TX failed; see {log}"
completion = next(line for line in text.splitlines() if "completion: interrupt events=" in line)
if intel:
    assert "events=0" not in completion or "messages=0" not in completion, f"{backend} raised no completion IRQ; see {log}"
assert "EXPOS_COMMAND_OK shutdown" in text, f"guest did not finish; see {log}"
