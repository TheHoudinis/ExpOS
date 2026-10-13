#!/usr/bin/env python3
"""Hardware negative controls; only disposable images, never physical disks."""
from pathlib import Path
import os
import re
import selectors
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build/isolation"


def run(name, cpu, fatal=False):
    state = BUILD / f"{name}-state.img"
    with state.open("wb") as image:
        image.truncate(4 * 1024 * 1024)
    commands = ["2222", "operator", "expos"]
    cases = [("WriteCode", "write-code", 14), ("DivideZero", "divide-zero", 0),
             ("BadOpcode", "invalid-opcode", 6), ("Escape", "escape", 14)]
    if "-nx" not in cpu:
        cases.append(("ExecuteData", "exec-data", 14))
    if not fatal and "-nx" not in cpu:
        commands += ["servicetest"]
    if fatal:
        cases = []
    for form, payload, _ in cases:
        commands += [f"mkform {form} executable", f"write {form} native:{payload}",
                     f"grant {form} execute", f"execute {form}"]
    payload = "log kernel-fault-test!" if fatal else "ac-log survived-isolation-probes"
    commands += ["mkform Survivor executable", f"write Survivor native:{payload}",
                 "grant Survivor execute", "execute Survivor", "shutdown", ""]
    process = subprocess.Popen([
        "qemu-system-x86_64", "-machine", "pc", "-cpu", cpu, "-smp", "4", "-m", "256M",
        "-vga", "std", "-global", "VGA.vgamem_mb=16", "-net", "none",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-drive", f"file={state},format=raw,if=ide,index=0",
        "-cdrom", str(BUILD / "probes.iso"), "-display", "none", "-serial", "stdio", "-no-reboot",
    ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    sent = False
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + 120
    fatal_seen = False
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(0.1):
                chunk = os.read(key.fd, 65536)
                if chunk:
                    output.extend(chunk)
                if not sent and b"EXPOS_BOOT_MODE_READY" in output:
                    process.stdin.write("\n".join(commands).encode())
                    process.stdin.flush()
                    sent = True
            if fatal and re.search(rb"EXPOS_KERNEL_FAULT[^\n]*cpl=0\r*\n", output):
                fatal_seen = True
                break
            if process.poll() is not None:
                output.extend(process.stdout.read())
                break
        else:
            raise AssertionError(f"{name} timed out")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        selector.close()
        (BUILD / f"{name}.log").write_bytes(output)
    text = output.decode(errors="replace").replace("\r", "")
    assert "[KERNEL PANIC]" not in text, text[-3000:]
    for probe in ("text-write", "rodata-write"):
        assert f"name={probe} cpu=0 blocked=true" in text
    assert "EXPOS_SMP_READY discovered=4 online=4" in text
    if "-nx" not in cpu:
        for probe in ("rodata-execute", "data-execute"):
            assert f"name={probe} cpu=0 blocked=true" in text
    if "-smap" not in cpu:
        assert re.search(r"name=smap-user-read cpu=[0-3] blocked=true", text)
        assert "smap=true" in text
    else:
        assert "nx=false smep=false form_wx=false smap=false" in text
        assert "name=smap-user-read" not in text
        assert "EXPOS_KERNEL_MAPPINGS wx=false" in text
    if fatal:
        assert fatal_seen and "EXPOS_KERNEL_FAULT vector=14 error=0x0" in text
        assert "EXPOS_FORM_FAULT" not in text
        assert "EXPOS_COMMAND_OK shutdown" not in text
    else:
        assert process.returncode == 33, text[-3000:]
        assert "EXPOS_KERNEL_FAULT" not in text
        if "-nx" not in cpu:
            for probe in ("opcode", "input-write", "watchdog"):
                assert f"EXPOS_SERVICE_PROBE name={probe} contained=true recovered=true" in text
        for form, _, vector in cases:
            assert f"Executable Form '{form}' faulted at vector {vector}." in text
        # Hardware page-fault reasons: user write to RO code and NX fetch.
        assert "vector=14 error=0x7" in text
        if "-nx" not in cpu:
            assert "vector=14 error=0x15" in text
        assert "Executable Form 'Survivor' exited with result 0 in Ring 3." in text
        assert "EXPOS_COMMAND_OK shutdown" in text
    print(f"ISOLATION {name}: passed", flush=True)


run("protected", "max")
run("legacy-cpu", "max,-nx,-smep,-smap")
run("supervisor-fault", "max", fatal=True)
