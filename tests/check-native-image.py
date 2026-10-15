#!/usr/bin/env python3
"""Compile contributor C/Rust, import/persist it, and exercise real CPL3 failures."""
from pathlib import Path
import os
import re
import selectors
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build/native-image"
BUILD.mkdir(parents=True, exist_ok=True)
CLI = ROOT / "sdk/bin/expos"
MANIFEST = ROOT / "sdk/examples/native-hello/expos.toml"
subprocess.run([str(CLI), "--manifest", str(MANIFEST), "build"], check=True)
script = subprocess.check_output([str(CLI), "--manifest", str(MANIFEST),
                                  "import-script", "--no-build"], text=True).splitlines()
assert all(len(line) < 128 for line in script)

RUST_MANIFEST = ROOT / "sdk/examples/native-rust/expos.toml"
subprocess.run([str(CLI), "--manifest", str(RUST_MANIFEST), "build"], check=True)
rust_script = subprocess.check_output([str(CLI), "--manifest", str(RUST_MANIFEST),
                                       "import-script", "--no-build"], text=True).splitlines()

GO_MANIFEST = ROOT / "sdk/examples/native-go/expos.toml"
subprocess.run([str(CLI), "--manifest", str(GO_MANIFEST), "build"], check=True)
go_script = subprocess.check_output([str(CLI), "--manifest", str(GO_MANIFEST),
                                     "import-script", "--no-build"], text=True).splitlines()


def image(code, data=b"", entry=0, version=1):
    return struct.pack("<4s8HI", b"EXF1", version, 1, 1, 0, len(code), len(data), 0, entry, 0) + code + data


def upload(payload, name="Probe"):
    return [f"writehex {name} {offset} {payload[offset:offset+32].hex()}"
            for offset in range(0, len(payload), 32)]


def run(name, commands, cpu="max", reset=True):
    state = BUILD / ("legacy.img" if "-nx" in cpu else "state.img")
    if reset:
        with state.open("wb") as output:
            output.truncate(4 * 1024 * 1024)
    process = subprocess.Popen([
        "qemu-system-x86_64", "-machine", "pc", "-cpu", cpu, "-smp", "4", "-m", "256M",
        "-vga", "std", "-global", "VGA.vgamem_mb=16", "-net", "none",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-drive", f"file={state},format=raw,if=ide,index=0", "-cdrom", str(ROOT / "build/expos.iso"),
        "-display", "none", "-serial", "stdio", "-no-reboot"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + 120
    sent = False
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(0.1):
                output.extend(os.read(key.fd, 65536))
            if not sent and b"EXPOS_BOOT_MODE_READY" in output:
                process.stdin.write(("\n".join(["2222", "operator", "expos", *commands, "shutdown", ""])).encode())
                process.stdin.flush()
                sent = True
            if process.poll() is not None:
                output.extend(process.stdout.read())
                break
        else:
            raise AssertionError(f"{name}: timed out")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        selector.close()
        (BUILD / f"{name}.log").write_bytes(output)
    text = output.decode(errors="replace").replace("\r", "")
    assert process.returncode == 33, f"{name}: see {BUILD / (name + '.log')}"
    assert "EXPOS_KERNEL_FAULT" not in text and "[KERNEL PANIC]" not in text
    assert "EXPOS_COMMAND_OK shutdown" in text
    assert "EXPOS_SMP_READY discovered=4 online=4" in text
    return text


# Reuse one Form so the test checks the image boundary, not the Form quota.
commands = ["expbudget set form-operations 512 1024", *script, *rust_script, *go_script,
            "mkform Probe executable", "grant Probe execute"]
faults = [
    (bytes.fromhex("31c08ee8dbe3"), 7),                       # x87 with CR0.TS
    (bytes.fromhex("0f57c0"), 6),                     # XORPS: OSFXSR disabled on BIOS
    (bytes.fromhex("f30faed8"), 6),                   # WRGSBASE disabled
    (bytes.fromhex("0f05"), 6),                       # SYSCALL disabled
    (bytes.fromhex("0f34"), (6, 13)),                 # SYSENTER has no entry selector
    (bytes.fromhex("fa"), 13),                       # CLI is privileged
    (bytes.fromhex("48b80000000000400000c60090"), 14), # write RX code
    (bytes.fromhex("48b80010000000400000ffe0"), 14),   # execute NX data
]
for code, _ in faults:
    commands += [*upload(image(code)), "execute Probe"]
# Mutating the user GS selector must not corrupt kernel GS on timer/ABI/fault.
commands += [*upload(image(bytes.fromhex(
    "31c08ee866c74702080089571448c747182b00000048b8494241534f505845cd800f0b"))), "execute Probe"]
# EXF1 format v2 crosses the old 4 KiB ceiling and must use independently
# allocated code pages plus guard-separated data/stack ranges.
v2_exit = bytes.fromhex(
    "4889fb48b80000010000400000c6002a66c74302080089531448c743184d0000004889df488d7350"
    "48b8494241534f505845cd800f0b"
)
v2_code = v2_exit + bytes([0x90]) * (4120 - len(v2_exit))
commands += [*upload(image(v2_code, version=2)), "execute Probe"]
# Preserve the preemption/budget proof after the v2 re-launch. A
# budget-blocked context deliberately remains non-runnable for inspection.
commands += [*upload(image(bytes.fromhex("31c08ee8f390ebfc"))), "execute Probe"]
# Exercise lifecycle transitions and queued IPC without letting a spinning
# background Form monopolize the command environment.
commands += ["mkform Background executable", "write Background native:spin",
             "grant Background execute", "formctl start Background",
             "formctl pause Background", "formctl resume Background",
             "formctl wait Background", "formctl send Background wake",
             "formctl terminate Background", "formctl restart Background",
             "formctl terminate Background"]
# Partial/malformed uploads never become executable and rejected writes are atomic.
commands += [*upload(b"EXF1"), "execute Probe", "writehex Probe 4 gg", "writehex Probe 5 00",
             "execute Probe", "execute NativeHello"]
text = run("protected", commands)
assert "Hello from compiled Rust in Ring 3" in text
assert "Executable Form 'NativeRust' exited with result 43 in Ring 3." in text
assert "Hello from native Go in Ring 3" in text
assert "Executable Form 'NativeGo' exited with result 43 in Ring 3." in text
assert text.count("Hello from compiled C in Ring 3") == 2
assert text.count("Executable Form 'NativeHello' exited with result 42 in Ring 3.") == 2
observed = [int(v) for v in re.findall(r"EXPOS_FORM_FAULT[^\n]*vector=(\d+)", text)]
assert len(observed) == len(faults), observed
for actual, (_, expected) in zip(observed, faults):
    assert actual in (expected if isinstance(expected, tuple) else (expected,)), (actual, expected)
assert "Executable Form 'Probe' exited with result 43 in Ring 3." in text
assert "Executable Form 'Probe' exited with result 77 in Ring 3." in text
assert "EXPOS_FORM_MAP" in text and "format=2 code_pages=2" in text
assert "heap_pages=16" in text and "guards=4" in text
assert "EXPOS_FRAME_ALLOCATOR" in text
assert "Started 'Background' as background context" in text
assert "Paused 'Background'" in text and "Queued asynchronous IPC for 'Background'" in text
assert text.count("Terminated 'Background'") == 2
assert "EXPOS_FORM_BUDGET_EXHAUSTED" in text
assert text.count("Native launch denied: invalid executable header.") == 2
assert "Invalid hex bytes; nothing changed." in text
assert "Invalid binary range; nothing changed." in text
assert "error=0x7" in text and "error=0x15" in text
print("NATIVE IMAGE protected: compiled C/Rust/Go, data/BSS, GS, preemption, faults and rejected imports passed", flush=True)

text = run("persistence", ["grant NativeHello execute", "execute NativeHello"], reset=False)
assert "Executable Form 'NativeHello' exited with result 42 in Ring 3." in text
print("NATIVE IMAGE persistence: saved executable survived reboot", flush=True)

text = run("legacy-cpu", [*script, "mkform Legacy executable", "write Legacy native:log legacy-survived",
                          "grant Legacy execute", "execute Legacy"], cpu="max,-nx,-smep,-smap")
assert "Native launch denied: native images require CPU NX protection." in text
assert "Hello from compiled C in Ring 3" not in text
assert "Executable Form 'Legacy' exited with result 0 in Ring 3." in text
print("NATIVE IMAGE legacy-cpu: image refused without NX; diagnostic capsules still work", flush=True)
