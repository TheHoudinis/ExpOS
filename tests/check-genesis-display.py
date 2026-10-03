#!/usr/bin/env python3
"""Prove the Genesis prompt is visible and accepts QEMU PS/2 input."""

from collections import Counter
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "build/genesis-display"


def read_ppm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    position = 0
    tokens = []
    while len(tokens) < 4:
        while data[position : position + 1].isspace():
            position += 1
        if data[position : position + 1] == b"#":
            position = data.index(b"\n", position) + 1
            continue
        end = position
        while end < len(data) and not data[end : end + 1].isspace():
            end += 1
        tokens.append(data[position:end])
        position = end
    if tokens[0] != b"P6" or tokens[3] != b"255":
        raise AssertionError(f"unsupported screenshot format: {path}")
    position += 2 if data[position : position + 2] == b"\r\n" else 1
    width, height = map(int, tokens[1:3])
    pixels = data[position:]
    if len(pixels) != width * height * 3:
        raise AssertionError(f"incomplete screenshot: {path}")
    return width, height, pixels


def inspect_genesis_picture(path: Path) -> bytes:
    width, height, pixels = read_ppm(path)
    assert 640 <= width <= 1920 and 480 <= height <= 1080, (
        f"unexpected Genesis size: {width}x{height}"
    )
    colors = Counter(pixels[index : index + 3] for index in range(0, len(pixels), 3))
    total = width * height
    assert total - colors[b"\0\0\0"] > total * 9 // 10, "Genesis did not replace OVMF scanout"
    for expected in (b"\x08\x0b\x10", b"\x0d\x11\x18", b"\x12\x18\x20", b"\x6f\xbf\x9a"):
        assert colors[expected] > 0, f"Genesis theme color {expected.hex()} is absent"
    assert len(colors) >= 7, f"Genesis prompt lacks rendered text: {len(colors)} colors"
    return pixels


def wait_for(serial: Path, marker: str, process: subprocess.Popen, deadline: float) -> None:
    while marker not in serial.read_text(errors="replace"):
        if process.poll() is not None:
            raise AssertionError(f"QEMU exited early ({process.returncode})")
        if time.monotonic() >= deadline:
            raise TimeoutError(f"Genesis never reached {marker!r}")
        time.sleep(0.04)


def qmp_connect(path: Path):
    connection = socket.socket(socket.AF_UNIX)
    connection.settimeout(3)
    connection.connect(str(path))
    wire = connection.makefile("rwb")
    greeting = json.loads(wire.readline())
    assert "QMP" in greeting

    def command(execute: str, **arguments):
        message = {"execute": execute}
        if arguments:
            message["arguments"] = arguments
        wire.write(json.dumps(message).encode() + b"\n")
        wire.flush()
        while True:
            response = json.loads(wire.readline())
            if "error" in response:
                raise AssertionError(f"QMP {execute}: {response['error']}")
            if "return" in response:
                return response["return"]

    command("qmp_capabilities")
    return connection, wire, command


def main() -> None:
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    serial = ARTIFACTS / "serial.log"
    serial.write_text("")
    with tempfile.TemporaryDirectory(prefix="expos-genesis-display-") as directory:
        scratch = Path(directory)
        disk = scratch / "target.img"
        with disk.open("wb") as stream:
            stream.truncate(128 * 1024 * 1024)
        variables = scratch / "vars.fd"
        shutil.copyfile(
            os.environ.get("OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd"),
            variables,
        )
        qmp_path = scratch / "qmp.sock"
        command = [
            "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
            "-vga", "std", "-global", "VGA.vgamem_mb=16", "-display", "none",
            "-drive", f"if=pflash,format=raw,readonly=on,file={os.environ.get('OVMF_CODE', '/usr/share/edk2/x64/OVMF_CODE.4m.fd')}",
            "-drive", f"if=pflash,format=raw,file={variables}",
            "-drive", f"file={disk},format=raw,if=ide,index=0",
            "-cdrom", str(ROOT / "build/ExpOS-v9-x86_64.iso"), "-boot", "once=d",
            "-serial", f"file:{serial}", "-qmp", f"unix:{qmp_path},server=on,wait=off",
            "-no-reboot",
        ]
        log = (ARTIFACTS / "qemu.log").open("w")
        process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        connection = wire = None
        deadline = time.monotonic() + 45
        try:
            wait_for(serial, "Installation mode [1/2]:", process, deadline)
            wait_for(serial, "EXPOS_GENESIS_DISPLAY visible=true backend=uefi-gop", process, deadline)
            connection, wire, qmp = qmp_connect(qmp_path)
            prompt = ARTIFACTS / "prompt.ppm"
            qmp("screendump", filename=str(prompt))
            prompt_pixels = inspect_genesis_picture(prompt)
            qmp("send-key", keys=[{"type": "qcode", "data": "1"}], **{"hold-time": 30})
            qmp("send-key", keys=[{"type": "qcode", "data": "ret"}], **{"hold-time": 30})
            wait_for(serial, "CFC name:", process, deadline)
            mode = ARTIFACTS / "mode-selected.ppm"
            qmp("screendump", filename=str(mode))
            mode_pixels = inspect_genesis_picture(mode)
            changed = sum(
                prompt_pixels[index : index + 3] != mode_pixels[index : index + 3]
                for index in range(0, len(prompt_pixels), 3)
            )
            assert changed >= 300, f"Genesis prompt did not visibly advance ({changed} pixels)"
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if wire is not None:
                wire.close()
            if connection is not None:
                connection.close()
            log.close()
    print("Genesis visible framebuffer prompt and PS/2 input passed")


if __name__ == "__main__":
    main()
