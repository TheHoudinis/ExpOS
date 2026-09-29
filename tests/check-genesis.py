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


def check_variant(name: str, mode: bytes, encrypted: bool) -> None:
    target = build / f"genesis-{name}-target.img"
    target.write_bytes(b"")
    with target.open("r+b") as disk:
        disk.truncate(96 * 1024 * 1024)

    install_command = base + firmware(f"{name}-install") + [
        "-drive", f"file={target},format=raw,if=ide,index=0",
        "-cdrom", "build/ExpOS-0.9-x86_64.iso", "-boot", "once=d",
    ]
    common = b"Test Fabric\nPrimary\ncorrect-horse\ncorrect-horse\nERASE\n\n"
    install_status, install_output = run(
        f"{name}-install",
        install_command,
        b"Installation mode [1/2]:",
        mode + b"\n" + common,
        240 if encrypted else 150,
    )
    assert install_status == 0, (
        f"{name} installer failed ({install_status}); "
        f"see build/genesis/{name}-install-serial.log"
    )
    for marker in (
        b"EXPOS_GENESIS_INSTALLED",
        b"EFI/BOOT/BOOTX64.EFI installed",
        b"primary/backup manifests and boot payload verified",
        b"Installation complete",
    ):
        assert marker in install_output, f"{name} installer omitted {marker!r}"

    installed_bytes = target.read_bytes()
    primary_manifest = installed_bytes[40 * 512 : 41 * 512]
    backup_manifest = installed_bytes[41 * 512 : 42 * 512]
    assert primary_manifest == backup_manifest, "Genesis manifest copies differ"
    assert primary_manifest[:8] == b"EXGEN001"
    if name == "architect":
        with target.open("r+b") as disk:
            disk.seek(40 * 512)
            disk.write(b"DAMAGED!")

    boot_command = base + firmware(f"{name}-boot") + [
        "-drive", f"file={target},format=raw,if=ide,index=0",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    ]
    if encrypted:
        ready = b"Unlock password:"
        boot_input = b"correct-horse\n2\noperator\ncorrect-horse\nshutdown\n"
        timeout = 180
    else:
        ready = b"EXPOS_BOOT_MODE_READY"
        boot_input = b"2\noperator\ncorrect-horse\nshutdown\n"
        timeout = 60
    boot_status, boot_output = run(
        f"{name}-boot", boot_command, ready, boot_input, timeout
    )
    assert boot_status == 33, (
        f"{name} installed boot failed ({boot_status}); "
        f"see build/genesis/{name}-boot-serial.log"
    )
    for marker in (
        b"EXPOS_ACCOUNTS_READY source=genesis persisted=true",
        b"EXPOS_EARLY_DISPLAY visible=true backend=bochs-vbe",
        b"EXPOS_EXPFS_BASELINE_READY",
        b"EXPOS_LOGIN_OK user=operator",
        b"EXPOS_COMMAND_OK shutdown",
    ):
        assert marker in boot_output, f"{name} installed boot omitted {marker!r}"
    if name == "architect":
        assert b"EXPOS_GENESIS_MANIFEST_RECOVERED source=backup" in boot_output
    disk_bytes = target.read_bytes()
    assert disk_bytes[41 * 512 : 41 * 512 + 8] == b"EXGEN001"
    baseline = 304 * 512
    assert disk_bytes[baseline : baseline + 8] == b"EXPFSDB1"
    if encrypted:
        assert b"EXPOS_CFC_UNLOCKED" in boot_output
        assert b"suite=xchacha20poly1305" in boot_output
        assert b"correct-horse" not in disk_bytes
        assert disk_bytes[64 * 512 + 44] == 1, "ExpFS slot A is not encrypted"
        assert disk_bytes[baseline + 44] == 1, "installation baseline is not encrypted"
    else:
        assert disk_bytes[baseline + 44] == 0, "Architect baseline should be unencrypted"


check_variant("architect", b"2", False)
check_variant("basic", b"1", True)
print("Genesis verified install, redundant manifest, protected baseline, disk boot, login, and shutdown completed")
