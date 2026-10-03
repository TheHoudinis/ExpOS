#!/usr/bin/env python3
"""Exercise modern storage and USB paths against real QEMU devices."""

import json
import os
from pathlib import Path
import selectors
import socket
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "build" / "modern-hardware"
ISO = ROOT / "build" / "expos.iso"
DEBUG_EXIT = ["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]


def base_command() -> list[str]:
    return [
        "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
        "-vga", "std", "-display", "none", "-boot", "once=d", "-cdrom",
        str(ISO), "-no-reboot",
    ]


def storage_arguments(backend: str, disk: Path) -> list[str]:
    drive = f"if=none,id=state,file={disk},format=raw"
    if backend == "nvme":
        return ["-drive", drive, "-device", "nvme,serial=EXPOSNVME0001,drive=state"]
    if backend == "ahci":
        return [
            "-device", "ich9-ahci,id=ahci", "-drive", drive,
            "-device", "ide-hd,drive=state,bus=ahci.0",
        ]
    raise ValueError(backend)


def storage_boot(backend: str, disk: Path, phase: str, commands: str) -> str:
    command = base_command() + storage_arguments(backend, disk) + DEBUG_EXIT + ["-serial", "stdio"]
    process = subprocess.Popen(
        command, cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    sent = False
    deadline = time.monotonic() + 45
    try:
        while time.monotonic() < deadline:
            for key, _ in selector.select(0.2):
                chunk = os.read(key.fd, 16384)
                if chunk:
                    output.extend(chunk)
                    if not sent and b"EXPOS_BOOT_MODE_READY" in output:
                        process.stdin.write(commands.encode())
                        process.stdin.flush()
                        sent = True
            if process.poll() is not None:
                output.extend(process.stdout.read())
                break
        else:
            raise TimeoutError(f"{backend} {phase} boot timed out")
    finally:
        selector.close()
        if process.poll() is None:
            process.kill()
            process.wait()
    log = ARTIFACTS / f"{backend}-{phase}.log"
    log.write_bytes(output)
    text = output.decode(errors="replace")
    assert process.returncode == 33, f"{backend} {phase} failed; see {log}"
    assert f"EXPOS_STORAGE_READY backend={backend}" in text, log
    assert "EXPOS_COMMAND_OK shutdown" in text, log
    return text


def check_storage(backend: str) -> None:
    disk = ARTIFACTS / f"{backend}-state.img"
    disk.write_bytes(b"")
    with disk.open("r+b") as stream:
        stream.truncate(8 * 1024 * 1024)
    marker = f"modern-{backend}-roundtrip"
    first = storage_boot(
        backend, disk, "write",
        f"2\noperator\nexpos\nmkform ModernDisk data\nwrite ModernDisk {marker}\nshutdown\n",
    )
    assert "Created and bound 'ModernDisk'" in first, f"{backend} did not persist a Form"
    assert "EXPOS_EXPFS_COMMIT generation=" in first, f"{backend} did not commit ExpFS"
    second = storage_boot(
        backend, disk, "read", "2\noperator\nexpos\ncat ModernDisk\nshutdown\n",
    )
    assert "EXPOS_EXPFS_FORMS_RESTORED" in second, f"{backend} did not restore Forms"
    assert marker in second, f"{backend} data did not survive reboot"


class Qmp:
    def __init__(self, path: Path, process: subprocess.Popen):
        deadline = time.monotonic() + 5
        while not path.exists():
            if process.poll() is not None:
                raise AssertionError(f"QEMU exited before QMP was ready ({process.returncode})")
            if time.monotonic() >= deadline:
                raise TimeoutError("QMP socket did not appear")
            time.sleep(0.02)
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(3)
        self.connection.connect(str(path))
        self.wire = self.connection.makefile("rwb")
        assert "QMP" in json.loads(self.wire.readline())
        self.command("qmp_capabilities")

    def command(self, execute: str, **arguments):
        message = {"execute": execute}
        if arguments:
            message["arguments"] = arguments
        self.wire.write(json.dumps(message).encode() + b"\n")
        self.wire.flush()
        while True:
            response = json.loads(self.wire.readline())
            if "error" in response:
                raise AssertionError(f"QMP {execute}: {response['error']}")
            if "return" in response:
                return response["return"]

    def type(self, text: str) -> None:
        for character in text:
            key = {"\n": "ret", " ": "spc"}.get(character, character)
            self.command(
                "send-key", keys=[{"type": "qcode", "data": key}], **{"hold-time": 30}
            )
            time.sleep(0.07)

    def close(self) -> None:
        self.wire.close()
        self.connection.close()


def wait_for(path: Path, marker: str, process: subprocess.Popen, deadline: float) -> None:
    while marker not in path.read_text(errors="replace"):
        if process.poll() is not None:
            raise AssertionError(f"QEMU exited before {marker!r} ({process.returncode})")
        if time.monotonic() >= deadline:
            raise TimeoutError(f"USB check never reached {marker!r}")
        time.sleep(0.03)


def check_usb_hid() -> None:
    serial = ARTIFACTS / "usb-hid.log"
    qemu_log = ARTIFACTS / "usb-hid-qemu.log"
    qmp_path = ARTIFACTS / "usb-hid-qmp.sock"
    disk = ARTIFACTS / "usb-state.img"
    serial.write_text("")
    qmp_path.unlink(missing_ok=True)
    disk.write_bytes(b"")
    with disk.open("r+b") as stream:
        stream.truncate(8 * 1024 * 1024)
    command = [
        "qemu-system-x86_64", "-machine", "pc,i8042=off", "-cpu", "max",
        "-m", "256M", "-vga", "std", "-display", "none",
        "-device", "qemu-xhci,id=xhci", "-device", "usb-kbd,bus=xhci.0",
        "-device", "usb-mouse,bus=xhci.0",
        "-drive", f"file={disk},format=raw,if=ide,index=0",
        "-serial", f"file:{serial}", "-qmp", f"unix:{qmp_path},server=on,wait=off",
        *DEBUG_EXIT, "-boot", "once=d", "-cdrom", str(ISO), "-no-reboot",
    ]
    with qemu_log.open("w") as log:
        process = subprocess.Popen(
            command, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT
        )
        qmp = None
        deadline = time.monotonic() + 45
        try:
            wait_for(serial, "EXPOS_BOOT_MODE_READY", process, deadline)
            qmp = Qmp(qmp_path, process)
            mice = qmp.command("query-mice")
            usb_mouse = next(
                (
                    mouse
                    for mouse in mice
                    if "usb" in mouse["name"].lower()
                    or "hid" in mouse["name"].lower()
                ),
                None,
            )
            assert usb_mouse is not None, f"QEMU did not expose the USB mouse: {mice}"
            if not usb_mouse["current"]:
                qmp.command(
                    "human-monitor-command",
                    **{"command-line": f"mouse_set {usb_mouse['index']}"},
                )
            # With i8042 disabled this motion can only arrive through usb-mouse.
            qmp.command(
                "input-send-event",
                events=[
                    {"type": "rel", "data": {"axis": "x", "value": 120}},
                    {"type": "rel", "data": {"axis": "y", "value": 10}},
                ],
            )
            time.sleep(0.2)
            # Select Console through the USB keyboard; no PS/2 controller is present.
            qmp.type("2\n")
            wait_for(serial, "EXPOS_BOOT_MODE console", process, deadline)
            qmp.type("operator\nexpos\nkstat\nasl\nshutdown\n")
            wait_for(serial, "EXPOS_COMMAND_OK shutdown", process, deadline)
            remaining = max(0.1, deadline - time.monotonic())
            assert process.wait(timeout=remaining) == 33, "USB guest did not shut down"
        finally:
            if qmp is not None:
                qmp.close()
            if process.poll() is None:
                process.kill()
                process.wait()
    text = serial.read_text(errors="replace")
    assert "EXPOS_USB_HID_DEVICE" in text and "protocol=keyboard" in text, serial
    assert "EXPOS_USB_HID_DEVICE" in text and "protocol=mouse" in text, serial
    assert "EXPOS_USB_READY controller=xhci keyboard=1 mouse=1 polling=true" in text, serial
    report_line = next(
        line for line in text.splitlines() if "input backends:" in line
    )
    assert "keyboard-reports=0" not in report_line, "usb-kbd produced no reports"
    assert "mouse-reports=0" not in report_line, "usb-mouse produced no reports"
    assert "EXPOS_COMMAND_OK asl" in text, "usb-kbd did not deliver shell input"


def main() -> None:
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    check_storage("nvme")
    print("NVMe read/write persistence passed")
    check_storage("ahci")
    print("AHCI read/write persistence passed")
    check_usb_hid()
    print("xHCI boot keyboard and boot mouse input passed with i8042 disabled")


if __name__ == "__main__":
    main()
