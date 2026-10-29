#!/usr/bin/env python3
"""Exercise AC'97 playback long enough to require a real completion IRQ."""

import os
from pathlib import Path
import selectors
import shlex
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
QEMU = [shlex.split(os.environ.get("QEMU", "qemu-system-x86_64"))[0]]

command = QEMU + [
    "-machine", "pc", "-cpu", "max", "-smp", "4", "-m", "256M",
    "-vga", "std", "-global", "VGA.vgamem_mb=16",
    "-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0",
    "-audiodev", f"driver=wav,id=exposaudio,path={BUILD / 'audio-output.wav'}",
    "-device", "AC97,audiodev=exposaudio",
    "-drive", f"file={BUILD / 'audio-state.img'},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-boot", "once=d", "-cdrom", str(BUILD / "expos.iso"),
    "-display", "none", "-serial", "stdio", "-no-reboot",
]

process = subprocess.Popen(
    command,
    cwd=ROOT,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output = bytearray()
started = False
finished_input = False
deadline = time.monotonic() + 90
try:
    # Open Settings/Audio, change volume and mute, then start the long tone.
    process.stdin.write(b"1111\noperator\nexpos\n~s\n6\nj\nj\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 16384)
            if chunk:
                output.extend(chunk)
        if not started and b"EXPOS_SETTING_CHANGED key=audio-test action=play" in output:
            started = True
            # The tone is 500 ms and crosses a descriptor boundary. Leave it
            # active long enough for QEMU to assert AC'97 completion INTx.
            time.sleep(1.5)
            process.stdin.write(b"~t\naudio\naudiostop\nshutdown\n")
            process.stdin.flush()
            finished_input = True
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError("audio QEMU run timed out")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    (BUILD / "audio-serial.log").write_bytes(output)

if not started or not finished_input:
    raise AssertionError("audio playback did not reach the IRQ wait point")
if process.returncode != 33:
    raise AssertionError(f"audio QEMU exited with {process.returncode}, expected 33")
