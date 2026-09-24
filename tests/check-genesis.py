#!/usr/bin/env python3
"""Install from the Genesis ISO, then boot and authenticate from that disk."""

import os
from pathlib import Path
import selectors
import shutil
import subprocess
import time

root = Path(__file__).resolve().parents[1]
build = root / "build/genesis"
code = os.environ.get("OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd")
vars_template = os.environ.get("OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd")
target = build / "genesis-check-target.img"
target.write_bytes(b"")
with target.open("r+b") as disk:
    disk.truncate(96 * 1024 * 1024)


def firmware(name: str) -> list[str]:
    variables = build / f"OVMF-{name}-vars.fd"
    shutil.copyfile(vars_template, variables)
    return [
        "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
        "-drive", f"if=pflash,format=raw,file={variables}",
    ]


def run(name: str, command: list[str], ready: bytes, supplied: bytes, timeout: int) -> tuple[int, bytes]:
    process = subprocess.Popen(
        command,
        cwd=root,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    sent = False
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(0.2):
                chunk = os.read(key.fd, 16384)
                if chunk:
                    output.extend(chunk)
                    if not sent and ready in output:
                        process.stdin.write(supplied)
                        process.stdin.flush()
                        sent = True
            if process.poll() is not None:
                output.extend(process.stdout.read())
                break
        else:
            raise TimeoutError(f"Genesis {name} phase timed out")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        selector.close()
        (build / f"{name}-serial.log").write_bytes(output)
    if not sent:
        raise AssertionError(f"Genesis {name} phase never reached readiness marker")
    return process.returncode, bytes(output)


base = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
    "-vga", "std", "-display", "none", "-serial", "stdio", "-no-reboot",
]
install_command = base + firmware("genesis-install") + [
    "-drive", f"file={target},format=raw,if=ide,index=0",
    "-cdrom", "build/ExpOS-0.9-x86_64.iso", "-boot", "once=d",
]
install_input = (root / "tests/qemu-genesis-input.txt").read_bytes() + b"\n"
install_status, install_output = run(
    "install", install_command, b"Installation mode [1/2]:", install_input, 90
)
assert install_status == 0, f"installer failed ({install_status}); see build/genesis/install-serial.log"
for marker in (
    b"EXPOS_GENESIS_INSTALLED",
    b"EFI/BOOT/BOOTX64.EFI installed",
    b"Installation complete",
):
    assert marker in install_output, f"installer omitted {marker!r}"

boot_command = base + firmware("genesis-boot") + [
    "-drive", f"file={target},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
]
boot_status, boot_output = run(
    "boot",
    boot_command,
    b"EXPOS_BOOT_MODE_READY",
    b"2\noperator\ncorrect-horse\nshutdown\n",
    60,
)
assert boot_status == 33, f"installed boot failed ({boot_status}); see build/genesis/boot-serial.log"
for marker in (
    b"EXPOS_ACCOUNTS_READY source=genesis persisted=true",
    b"EXPOS_LOGIN_OK user=operator",
    b"EXPOS_COMMAND_OK shutdown",
):
    assert marker in boot_output, f"installed boot omitted {marker!r}"
print("Genesis ISO install, disk boot, Operator login, and shutdown completed")
