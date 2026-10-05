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


def run_ps2(
    name: str,
    command: list[str],
    serial: Path,
    steps: list[tuple[bytes, bytes]],
    timeout: int,
) -> tuple[int, bytes]:
    serial.write_bytes(b"")
    process = subprocess.Popen(
        command,
        cwd=root,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    step = 0
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            output = serial.read_bytes()
            if step < len(steps) and steps[step][0] in output:
                supplied = steps[step][1]
                print(
                    f"{name}: PS/2 step {step + 1}/{len(steps)} at {steps[step][0]!r}",
                    flush=True,
                )
                names = {b"\n": "ret", b"-": "minus"}
                for byte in supplied:
                    key = names.get(bytes([byte]), chr(byte))
                    process.stdin.write(f"sendkey {key}\n".encode())
                    process.stdin.flush()
                    time.sleep(0.15)
                step += 1
            if process.poll() is not None:
                break
            time.sleep(0.03)
        else:
            raise TimeoutError(f"Genesis {name} phase timed out")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        output = serial.read_bytes()
        (build / f"{name}-serial.log").write_bytes(output)
    if step != len(steps):
        monitor = process.stdout.read().decode(errors="replace")
        raise AssertionError(
            f"Genesis {name} completed {step}/{len(steps)} PS/2 steps; monitor={monitor!r}"
        )
    return process.returncode, output


base = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4", "-m", "256M",
    "-vga", "std", "-display", "none", "-no-reboot",
]


def check_variant(
    name: str,
    mode: bytes,
    encrypted: bool,
    install_answers: bytes | None = None,
    expected_dimensions: int = 3,
    expected_users: int = 2,
    expected_components: int = 3,
    expected_sdks: int = 0,
    expected_drivers: int = 1,
    expected_apps: int = 13,
) -> None:
    target = build / f"genesis-{name}-target.img"
    target.write_bytes(b"")
    with target.open("r+b") as disk:
        disk.truncate(96 * 1024 * 1024)

    install_command = base + firmware(f"{name}-install") + [
        "-drive", f"file={target},format=raw,if=ide,index=0",
        "-cdrom", "build/ExpOS-v9-x86_64.iso", "-boot", "once=d",
        "-serial", "stdio",
    ]
    common = install_answers or (
        b"Test Fabric\nPrimary\n"
        b"1\n"                 # Unencrypted Architect / Easy Basic
        b"2\n"                 # Browser + three essential Ayo apps
        b"1\n"                 # No SDK contracts
        b"1\n"                 # Enable the detected network driver
        b"2\nWork\nLab\n"      # Two secondary Dimensions
        b"1\nanalyst\n2\n"     # One Guest account
        b"analyst-pass\nanalyst-pass\n"
        b"correct-horse\ncorrect-horse\nERASE\n\n"
    )
    install_status, install_output = run(
        f"{name}-install",
        install_command,
        b"Language [1-5]:",
        b"1\n" + mode + b"\n" + common,
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
    primary_plan = installed_bytes[42 * 512 : 43 * 512]
    backup_plan = installed_bytes[43 * 512 : 44 * 512]
    assert primary_manifest == backup_manifest, "Genesis manifest copies differ"
    assert primary_manifest[:8] == b"EXGEN001"
    assert primary_plan == backup_plan, "Genesis plan copies differ"
    assert primary_plan[:8] == b"EXPLAN02"
    assert primary_plan[11] == expected_components
    assert primary_plan[12] == expected_drivers
    assert primary_plan[13] == expected_sdks
    assert int.from_bytes(primary_plan[16:20], "little") == expected_apps
    if name == "architect":
        with target.open("r+b") as disk:
            disk.seek(40 * 512)
            disk.write(b"DAMAGED!")

    boot_command = base + firmware(f"{name}-boot") + [
        "-drive", f"file={target},format=raw,if=ide,index=0",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    ]
    guest_serial = build / f"{name}-boot-guest.log"
    boot_command += ["-serial", f"file:{guest_serial}", "-monitor", "stdio"]
    if encrypted:
        boot_steps = [(b"Unlock password:", b"correct-horse\n")]
        timeout = 300
    else:
        boot_steps = []
        timeout = 60
    boot_steps += [
        (b"EXPOS_BOOT_MODE_READY", b"2\n"),
        (b"EXPOS_LOGIN_READY", b"operator\n"),
        (b"user: operator", b"correct-horse\n"),
        (b"EXPOS_SHELL_READY", b"shutdown\n"),
    ]
    boot_status, boot_output = run_ps2(
        f"{name}-boot", boot_command, guest_serial, boot_steps, timeout
    )
    assert boot_status == 33, (
        f"{name} installed boot failed ({boot_status}); "
        f"see build/genesis/{name}-boot-serial.log"
    )
    for marker in (
        b"EXPOS_ACCOUNTS_READY source=genesis persisted=true",
        b"EXPOS_EARLY_DISPLAY visible=true backend=uefi-gop",
        b"EXPOS_EXPFS_BASELINE_READY",
        (
            f"EXPOS_GENESIS_PLAN_READY generation=1 dimensions={expected_dimensions} "
            f"users={expected_users} apps={expected_apps.bit_count()}"
        ).encode(),
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
        assert b"analyst-pass" not in disk_bytes
        assert disk_bytes[64 * 512 + 44] == 1, "ExpFS slot A is not encrypted"
        assert disk_bytes[baseline + 44] == 1, "installation baseline is not encrypted"
    else:
        assert disk_bytes[baseline + 44] == 0, "Architect baseline should be unencrypted"


selected_variant = os.environ.get("GENESIS_VARIANT", "both")
if selected_variant in ("both", "architect"):
    check_variant("architect", b"2", False)
if selected_variant in ("both", "basic"):
    check_variant("basic", b"1", True)
if selected_variant == "custom":
    check_variant(
        "custom",
        b"2",
        False,
        install_answers=(
            b"Custom Fabric\nPrimary\n"
            b"1\n"             # Unencrypted Architect
            b"4\n"             # Custom packages
            b"1\n"             # Browser installed
            b"2\n"             # ExpPython omitted
            b"1\n20\n0\n"      # Calculator + HashLab
            b"2\n"             # Rust + C SDK contracts
            b"2\n"             # Network driver omitted
            b"0\n"             # No secondary Dimensions
            b"0\n"             # No secondary users
            b"correct-horse\ncorrect-horse\nERASE\n\n"
        ),
        expected_dimensions=1,
        expected_users=1,
        expected_components=3,
        expected_sdks=3,
        expected_drivers=0,
        expected_apps=(1 << 0) | (1 << 19),
    )
print("Genesis verified install, redundant plan, protected baseline, PS/2 disk boot, login, and shutdown completed")
