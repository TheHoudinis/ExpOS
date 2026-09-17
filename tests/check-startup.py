#!/usr/bin/env python3
"""Verify actual scanout and PS/2 startup, not just successful serial messages.

Run after building `uefi` and `iso`. Screenshots and logs remain in
build/startup-{uefi,bios}; guest disks and firmware variables are disposable.
"""

import argparse
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


def read_ppm(path):
    """Read QEMU's binary P6 output without treating pixel bytes as whitespace."""
    data = path.read_bytes()
    position = 0
    tokens = []
    while len(tokens) < 4:
        while data[position:position + 1].isspace():
            position += 1
        if data[position:position + 1] == b"#":
            position = data.index(b"\n", position) + 1
            continue
        end = position
        while end < len(data) and not data[end:end + 1].isspace():
            end += 1
        tokens.append(data[position:end])
        position = end
    if tokens[0] != b"P6" or tokens[3] != b"255":
        raise AssertionError(f"Unsupported screenshot format: {path}")
    if data[position:position + 2] == b"\r\n":
        position += 2
    else:
        position += 1
    width, height = map(int, tokens[1:3])
    pixels = data[position:]
    if len(pixels) != width * height * 3:
        raise AssertionError(f"Incomplete screenshot: {path}")
    return width, height, pixels


def check_picture(path, graphical, previous=None):
    width, height, pixels = read_ppm(path)
    colors = Counter(pixels[index:index + 3] for index in range(0, len(pixels), 3))
    total = width * height
    nonblack = total - colors[b"\0\0\0"]
    if graphical:
        assert (width, height) == (640, 480), f"Unexpected GUI size: {width}x{height}"
        # A stale OVMF logo has many colors and some lit pixels. ExpOS fills
        # the screen; requiring a majority rejects that real failure too.
        assert nonblack > total // 2, f"Blank/stale GUI scanout: {nonblack}/{total} lit pixels"
        assert len(colors) >= 8, f"GUI lacks rendered content: {len(colors)} colors"
    else:
        assert (width, height) in ((640, 400), (720, 400)), "VGA text mode was not restored"
        assert nonblack >= 200 and len(colors) >= 2, "Console text is invisible"
    if previous is not None and len(previous) == len(pixels):
        changed = sum(pixels[index:index + 3] != previous[index:index + 3]
                      for index in range(0, len(pixels), 3))
        assert changed >= 500, f"Scanout did not change between screens ({changed} pixels)"
    return pixels, {"width": width, "height": height, "lit_pixels": nonblack,
                    "colors": len(colors)}


class Startup:
    def __init__(self, firmware, scratch, deadline):
        self.deadline = deadline
        self.artifacts = ROOT / "build" / f"startup-{firmware}"
        self.artifacts.mkdir(parents=True, exist_ok=True)
        self.serial = self.artifacts / "serial.log"
        self.serial.write_text("")
        self.qmp_path = scratch / "qmp.sock"
        state = scratch / "state.img"
        with state.open("wb") as stream:
            stream.truncate(4 * 1024 * 1024)
        command = ["qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-m", "256M",
                   "-vga", "std", "-global", "VGA.vgamem_mb=16", "-display", "none",
                   "-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0",
                   "-drive", f"file={state},format=raw,if=ide,index=0",
                   "-serial", f"file:{self.serial}",
                   "-qmp", f"unix:{self.qmp_path},server=on,wait=off",
                   "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-no-reboot"]
        if firmware == "uefi":
            code = os.environ.get("OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd")
            variables = os.environ.get("OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd")
            shutil.copyfile(variables, scratch / "vars.fd")
            # Keep firmware and the writable virtual FAT backend away from
            # both runtime state and the build's original boot payload.
            source = Path(os.environ.get("EXPOS_EFI_DIR", str(ROOT / "build/esp")))
            shutil.copytree(source, scratch / "esp")
            command += ["-drive", f"if=pflash,format=raw,readonly=on,file={code}",
                        "-drive", f"if=pflash,format=raw,file={scratch}/vars.fd",
                        "-drive", f"file=fat:rw:{scratch}/esp,format=raw,if=ide,index=1"]
        else:
            command += ["-boot", "once=d", "-cdrom", str(ROOT / "build/expos.iso")]
        self.log = (self.artifacts / "qemu.log").open("w")
        self.process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                                        stdout=self.log, stderr=subprocess.STDOUT)
        self.connection = None
        self.wire = None
        self.pictures = {}

    def check_alive(self):
        if self.process.poll() is not None:
            raise AssertionError(f"QEMU exited early ({self.process.returncode})")
        if time.monotonic() >= self.deadline:
            raise TimeoutError("Startup check exceeded 45 seconds")

    def wait_for(self, marker, count=1):
        while self.serial.read_text(errors="replace").count(marker) < count:
            self.check_alive()
            time.sleep(0.03)

    def connect(self):
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(3)
        self.connection.connect(str(self.qmp_path))
        self.wire = self.connection.makefile("rwb")
        greeting = json.loads(self.wire.readline())
        assert "QMP" in greeting, "Missing QMP greeting"
        self.qmp("qmp_capabilities")

    def qmp(self, execute, **arguments):
        self.check_alive()
        self.wire.write(json.dumps({"execute": execute, "arguments": arguments}).encode() + b"\n")
        self.wire.flush()
        while True:
            reply = self.wire.readline()
            if not reply:
                raise AssertionError("QMP disconnected")
            response = json.loads(reply)
            if "error" in response:
                raise AssertionError(f"QMP {execute}: {response['error']}")
            if "return" in response:
                return response["return"]

    def type(self, text):
        # No serial input: every character goes through QEMU's PS/2 keyboard.
        for character in text:
            name = {"\n": "ret", " ": "spc"}.get(character, character)
            self.qmp("send-key", keys=[{"type": "qcode", "data": name}], **{"hold-time": 30})
            time.sleep(0.08)

    def picture(self, name, graphical=True, previous=None):
        # QEMU's UI refresh can lag a completed guest frame by one timer tick.
        time.sleep(0.2)
        path = self.artifacts / f"{name}.ppm"
        self.qmp("screendump", filename=str(path))
        pixels, stats = check_picture(path, graphical, previous)
        self.pictures[name] = stats
        return pixels

    def run(self):
        self.wait_for("EXPOS_BOOT_MODE_READY")
        self.connect()
        chooser = self.picture("chooser")
        self.type("1\n")
        self.wait_for("EXPOS_LOGIN_SCREEN_PRESENTED")
        login = self.picture("graphical-login", previous=chooser)
        self.type("operator\nexpos\n")
        self.wait_for("EXPOS_DESKTOP_EMPTY")
        self.picture("desktop", previous=login)
        self.type("q")
        self.wait_for("EXPOS_SHELL_READY")
        self.picture("console", graphical=False)

        self.type("login\n")
        self.wait_for("EXPOS_BOOT_MODE_READY", count=2)
        self.type("2\n")
        self.wait_for("EXPOS_LOGIN_READY", count=2)
        console_login = self.picture("console-login", graphical=False)
        self.type("operator\nexpos\n")
        self.wait_for("EXPOS_LOGIN_OK user=operator", count=2)
        self.wait_for("EXPOS_COMMAND_OK login")
        self.picture("console-authenticated", graphical=False, previous=console_login)
        self.type("shutdown\n")
        self.wait_for("EXPOS_COMMAND_OK shutdown")
        remaining = max(0.1, self.deadline - time.monotonic())
        assert self.process.wait(timeout=remaining) == 33, "Guest did not shut down cleanly"
        (self.artifacts / "screens.json").write_text(json.dumps(self.pictures, indent=2) + "\n")

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if self.wire is not None:
            self.wire.close()
        if self.connection is not None:
            self.connection.close()
        self.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("firmware", choices=("uefi", "bios", "both"), nargs="?", default="both")
    args = parser.parse_args()
    deadline = time.monotonic() + 45
    for firmware in (("uefi", "bios") if args.firmware == "both" else (args.firmware,)):
        with tempfile.TemporaryDirectory(prefix="expos-startup-") as directory:
            startup = Startup(firmware, Path(directory), deadline)
            try:
                startup.run()
            except Exception as error:
                raise SystemExit(f"ExpOS {firmware} startup failed: {error}; see {startup.artifacts}") from error
            finally:
                startup.close()
            print(f"ExpOS {firmware}: visible chooser, graphical login, desktop, console login, and PS/2 shutdown passed")


if __name__ == "__main__":
    main()
