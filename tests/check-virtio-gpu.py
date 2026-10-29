#!/usr/bin/env python3
"""Exercise the modern VirtIO GPU 2D scanout and inspect a rendered frame."""

from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "build" / "virtio-gpu"


def wait_for(path: Path, marker: str, process: subprocess.Popen, deadline: float) -> str:
    while time.monotonic() < deadline:
        text = path.read_text(errors="replace")
        if marker in text:
            return text
        if process.poll() is not None:
            raise AssertionError(f"QEMU exited early ({process.returncode}) before {marker}")
        time.sleep(0.05)
    raise TimeoutError(f"Timed out waiting for {marker}")


def inspect_picture(path: Path) -> dict:
    data = path.read_bytes()
    header, width, height, maximum, pixels = data.split(maxsplit=4)
    assert header == b"P6" and maximum == b"255", "unexpected screenshot format"
    width, height = int(width), int(height)
    assert len(pixels) == width * height * 3, "incomplete VirtIO GPU screenshot"
    colors = Counter(pixels[index:index + 3] for index in range(0, len(pixels), 3))
    lit = width * height - colors[b"\0\0\0"]
    assert (width, height) == (640, 480), f"unexpected initial GPU mode {width}x{height}"
    assert lit > width * height // 4, "VirtIO GPU scanout is blank or stale"
    assert len(colors) >= 8, "VirtIO GPU scanout lacks composed desktop content"
    return {"width": width, "height": height, "lit_pixels": lit, "colors": len(colors)}


def main():
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    serial_path = ARTIFACTS / "serial.log"
    qemu_path = ARTIFACTS / "qemu.log"
    picture_path = ARTIFACTS / "desktop.ppm"
    summary_path = ARTIFACTS / "summary.json"
    serial_path.write_text("")
    if picture_path.exists():
        picture_path.unlink()
    deadline = time.monotonic() + 180
    with tempfile.TemporaryDirectory(prefix="expos-virtio-gpu-") as directory:
        scratch = Path(directory)
        state = scratch / "state.img"
        with state.open("wb") as stream:
            stream.truncate(4 * 1024 * 1024)
        command = [
            "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4",
            "-m", "256M", "-vga", "none", "-device", "virtio-gpu-pci",
            "-drive", f"file={state},format=raw,if=ide,index=0",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-boot", "once=d", "-cdrom", str(ROOT / "build" / "expos.iso"),
            "-display", "none", "-serial", "mon:stdio", "-no-reboot",
        ]
        input_bytes = (ROOT / "tests" / "qemu-display-input.txt").read_bytes()
        initial_input = b"1111\noperator\nexpos\n"
        assert input_bytes.startswith(initial_input)
        with serial_path.open("wb") as serial_stream, qemu_path.open("wb") as qemu_stream:
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                stdin=subprocess.PIPE,
                stdout=serial_stream,
                stderr=qemu_stream,
            )
            try:
                wait_for(serial_path, "EXPOS_BOOT_MODE_READY", process, deadline)
                process.stdin.write(initial_input)
                process.stdin.flush()
                wait_for(serial_path, "EXPOS_DISPLAY_READY surfaces=12 commit=12", process, deadline)
                # mon:stdio starts on the serial side. Ctrl-A C switches to the
                # human monitor without requiring a sandbox-blocked socket.
                process.stdin.write(b"\x01c")
                process.stdin.flush()
                time.sleep(0.15)
                process.stdin.write(f"screendump {picture_path}\n".encode())
                process.stdin.flush()
                while (not picture_path.exists() or picture_path.stat().st_size == 0):
                    if time.monotonic() >= deadline:
                        raise TimeoutError("VirtIO GPU screenshot was not produced")
                    time.sleep(0.03)
                picture = inspect_picture(picture_path)
                process.stdin.write(b"\x01c")
                process.stdin.write(input_bytes[len(initial_input):])
                process.stdin.flush()
                process.stdin.close()
                remaining = max(0.1, deadline - time.monotonic())
                status = process.wait(timeout=remaining)
                assert status == 33, f"guest did not shut down cleanly ({status})"
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()

    serial = serial_path.read_text(errors="replace")
    required = (
        "EXPOS_VIRTIO_GPU_READY transport=modern-pci mode=2d",
        "EXPOS_RENDER_CPU_ACCEL fill=rep-stosq copy=rep-movsq",
        "preset=480p backend=virtio-gpu",
        "preset=720p backend=virtio-gpu",
        "preset=1080p backend=virtio-gpu",
        "EXPOS_VIRTIO_GPU_STOP transfers=",
        "EXPOS_COMPOSITOR_COALESCE source=input max_deferred=7 policy=latest-state",
        "EXPOS_FRAME_WAIT mode=pit-one-shot",
    )
    for marker in required:
        assert marker in serial, f"missing runtime marker: {marker}"
    assert "KERNEL PANIC" not in serial
    presentation = re.findall(
        r"EXPOS_PRESENTATION_STATS frames=(\d+) missed=(\d+) idle=(\d+).*coalesced_compositions=(\d+)",
        serial,
    )
    rendering = re.findall(
        r"EXPOS_RENDER_STATS full=(\d+) damaged=(\d+).*copied_pixels=(\d+).*backdrop_hits=(\d+) backdrop_captures=(\d+)",
        serial,
    )
    assert presentation and rendering, "missing compositor performance telemetry"
    frames, missed, idle, coalesced = map(int, presentation[-1])
    full, damaged, copied_pixels, backdrop_hits, backdrop_captures = map(int, rendering[-1])
    assert coalesced >= 32, "scripted input burst did not exercise frame coalescing"
    assert frames <= 80, f"obsolete input states produced too many frames ({frames})"
    assert missed <= 5_000, f"compositor pacing regressed ({missed} missed frames)"
    assert copied_pixels <= 30_000_000, f"damage upload regressed ({copied_pixels} pixels)"
    assert backdrop_hits > 0 and backdrop_captures > 0, "wallpaper cache was not exercised"
    performance = {
        "frames": frames,
        "missed": missed,
        "idle": idle,
        "coalesced_compositions": coalesced,
        "full_compositions": full,
        "damaged_compositions": damaged,
        "copied_pixels": copied_pixels,
        "backdrop_hits": backdrop_hits,
        "backdrop_captures": backdrop_captures,
    }
    summary_path.write_text(
        json.dumps({"picture": picture, "performance": performance, "markers": list(required)}, indent=2) + "\n"
    )
    print("VirtIO GPU scanout, cached composition, bounded input coalescing, and visible rendering passed")


if __name__ == "__main__":
    main()
