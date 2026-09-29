#!/usr/bin/env python3
"""Run Genesis visibly, ejecting installer media before its first reboot.

The selected path is always a regular image file. A missing image is created
as a sparse 128 MiB blank target. An image with a Genesis manifest boots from
disk without attaching the installer ISO; unknown nonblank images are refused.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_OFFSET = 40 * 512
MANIFEST_MAGIC = b"EXGEN001"
DEFAULT_SIZE_MIB = 128


def installed(path: Path) -> bool:
    with path.open("rb") as stream:
        stream.seek(MANIFEST_OFFSET)
        return stream.read(len(MANIFEST_MAGIC)) == MANIFEST_MAGIC


def blank(path: Path) -> bool:
    with path.open("rb") as stream:
        remaining = 1024 * 1024
        while remaining:
            chunk = stream.read(min(65536, remaining))
            if not chunk:
                break
            if any(chunk):
                return False
            remaining -= len(chunk)
    return True


def qmp_connect(path: Path, process: subprocess.Popen, deadline: float):
    while not path.exists():
        if process.poll() is not None:
            raise RuntimeError(f"QEMU exited before QMP became ready ({process.returncode})")
        if time.monotonic() >= deadline:
            raise TimeoutError("QMP socket did not appear")
        time.sleep(0.03)
    connection = socket.socket(socket.AF_UNIX)
    connection.settimeout(3)
    connection.connect(str(path))
    wire = connection.makefile("rwb")
    greeting = json.loads(wire.readline())
    if "QMP" not in greeting:
        raise RuntimeError("QEMU returned an invalid QMP greeting")

    def command(execute: str, **arguments):
        request = {"execute": execute}
        if arguments:
            request["arguments"] = arguments
        wire.write(json.dumps(request).encode() + b"\n")
        wire.flush()
        while True:
            response = json.loads(wire.readline())
            if "error" in response:
                raise RuntimeError(f"QMP {execute}: {response['error']}")
            if "return" in response:
                return response["return"]

    command("qmp_capabilities")
    return connection, wire, command


def eject_optical_media(qmp) -> bool:
    for block in qmp("query-block"):
        if not block.get("removable") or not block.get("inserted"):
            continue
        qmp("eject", device=block["device"], force=True)
        return True
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--disk",
        type=Path,
        default=Path(os.environ.get("GENESIS_DISK", ROOT / "build/genesis-interactive.img")),
        help="regular image file used as the isolated target",
    )
    parser.add_argument("--size-mib", type=int, default=DEFAULT_SIZE_MIB)
    args = parser.parse_args()
    disk = args.disk.expanduser().resolve()
    if disk.exists() and not disk.is_file():
        raise SystemExit(f"refusing non-regular target: {disk}")
    if not disk.exists():
        disk.parent.mkdir(parents=True, exist_ok=True)
        with disk.open("wb") as stream:
            stream.truncate(args.size_mib * 1024 * 1024)
        print(f"Created isolated {args.size_mib} MiB target: {disk}", flush=True)
    elif not installed(disk) and not blank(disk):
        raise SystemExit(
            f"refusing unknown nonblank image: {disk}\n"
            "Choose a new GENESIS_DISK path; this runner never guesses or erases it."
        )

    boot_installer = not installed(disk)
    iso = ROOT / "build/ExpOS-0.9-x86_64.iso"
    if boot_installer and not iso.is_file():
        raise SystemExit(f"missing installer ISO: {iso}")
    ovmf_code = Path(os.environ.get("OVMF_CODE", "/usr/share/edk2/x64/OVMF_CODE.4m.fd"))
    ovmf_template = Path(os.environ.get("OVMF_VARS", "/usr/share/edk2/x64/OVMF_VARS.4m.fd"))
    variables = disk.with_name(f"{disk.stem}-ovmf-vars.fd")
    if not variables.exists():
        shutil.copyfile(ovmf_template, variables)
    serial = disk.with_name(f"{disk.stem}-serial.log")
    serial.write_text("")

    with tempfile.TemporaryDirectory(prefix="expos-genesis-run-") as directory:
        qmp_path = Path(directory) / "qmp.sock"
        command = [
            "qemu-system-x86_64", "-name", "ExpOS Genesis", "-machine", "pc",
            "-cpu", "max", "-m", "256M", "-vga", "std", "-global", "VGA.vgamem_mb=16",
            "-netdev", "user,id=net0", "-device", "rtl8139,netdev=net0",
            "-drive", f"if=pflash,format=raw,readonly=on,file={ovmf_code}",
            "-drive", f"if=pflash,format=raw,file={variables}",
            "-drive", f"file={disk},format=raw,if=ide,index=0",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-display", "gtk,zoom-to-fit=on", "-serial", f"file:{serial}",
            "-qmp", f"unix:{qmp_path},server=on,wait=off",
        ]
        if boot_installer:
            command += ["-cdrom", str(iso), "-boot", "once=d"]
            print("Booting the visible installer; media will auto-eject at completion.", flush=True)
        else:
            print("Installed Genesis manifest found; booting the virtual disk directly.", flush=True)
        process = subprocess.Popen(command, cwd=ROOT)
        connection = wire = None
        try:
            connection, wire, qmp = qmp_connect(qmp_path, process, time.monotonic() + 10)
            ejected = not boot_installer
            while process.poll() is None:
                if not ejected and "Installation complete." in serial.read_text(errors="replace"):
                    if not eject_optical_media(qmp):
                        raise RuntimeError("Genesis completed but no removable installer media was found")
                    ejected = True
                    print("Installer media ejected. Press Enter in Genesis to boot the installed disk.", flush=True)
                time.sleep(0.05)
            returncode = process.returncode
            return 0 if returncode in (0, 33) else returncode
        except KeyboardInterrupt:
            return 130
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


if __name__ == "__main__":
    raise SystemExit(main())
