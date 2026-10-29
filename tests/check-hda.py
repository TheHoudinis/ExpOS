#!/usr/bin/env python3
"""Exercise Intel HDA playback and require a real stream completion IRQ."""

import os
from pathlib import Path
import selectors
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
disk, wav, log = BUILD / "hda-state.img", BUILD / "hda-output.wav", BUILD / "hda-serial.log"
disk.write_bytes(b"")
with disk.open("r+b") as stream:
    stream.truncate(8 * 1024 * 1024)
wav.unlink(missing_ok=True)
command = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4",
    "-m", "256M", "-vga", "std", "-display", "none",
    "-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0",
    "-audiodev", f"driver=wav,id=audio0,path={wav}",
    "-device", "intel-hda", "-device", "hda-output,audiodev=audio0",
    "-drive", f"file={disk},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-boot", "once=d",
    "-cdrom", str(BUILD / "expos.iso"), "-serial", "stdio", "-no-reboot",
]
process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output, sent, deadline = bytearray(), False, time.monotonic() + 90
try:
    # Enter the desktop, open Settings/Audio and activate the real test tone.
    process.stdin.write(b"1111\noperator\nexpos\n~s\n6\nj\nj\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 16384)
            if chunk:
                output.extend(chunk)
        if not sent and b"EXPOS_AUDIO_PLAY codec=generated-tone" in output:
            time.sleep(1.0)
            process.stdin.write(b"~t\naudio\naudiostop\nshutdown\n")
            process.stdin.flush()
            sent = True
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError("HDA guest timed out")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    log.write_bytes(output)
text = output.decode(errors="replace")
assert sent and process.returncode == 33, f"HDA guest failed; see {log}"
assert "EXPOS_HDA_READY" in text and "completion=interrupt" in text, log
status = next(line for line in text.splitlines() if "EXPOS_AUDIO_STATUS backend=hda" in line)
assert "interrupts=0" not in status and "completed=0" not in status, status
assert wav.stat().st_size > 44, "HDA WAV backend captured no PCM"
