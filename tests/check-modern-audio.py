#!/usr/bin/env python3
"""Download, persist, and play both external MP3 packages through VirtIO Sound."""

import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import selectors
import subprocess
import threading
import time


ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build"
SOURCES = {
    "/IMTQ.mp3": (
        Path(os.environ.get("EXPOS_IMTQ_MP3", "/home/houdini/Music/IMTQ.mp3")),
        7_585_316,
        "378ebe91fe508505353894c12a69a0a461cd257a101b3a7ff8e890b925d3f185",
    ),
    "/dash.mp3": (
        Path(os.environ.get("EXPOS_DASH_MP3", "/home/houdini/Music/-_-.mp3")),
        2_256_476,
        "fc71e866c57fca5126fab0e242538f531b121f6d13354a8858b2b6b0721c72f6",
    ),
}

for source, expected_size, expected_digest in SOURCES.values():
    assert source.is_file(), f"external test track missing: {source}"
    assert source.stat().st_size == expected_size, source
    assert hashlib.sha256(source.read_bytes()).hexdigest() == expected_digest, source


class RangeHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def do_GET(self):
        item = SOURCES.get(self.path)
        if item is None:
            self.send_error(404)
            return
        source, length, _ = item
        requested = self.headers.get("Range", "")
        if not requested.startswith("bytes=") or "," in requested:
            self.send_error(416)
            return
        try:
            first, last = requested[6:].split("-", 1)
            start, end = int(first), int(last)
        except (TypeError, ValueError):
            self.send_error(416)
            return
        if start < 0 or end < start or end >= length:
            self.send_error(416)
            return
        count = end - start + 1
        self.send_response(206)
        self.send_header("Content-Type", "audio/mpeg")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Range", f"bytes {start}-{end}/{length}")
        self.send_header("Content-Length", str(count))
        self.end_headers()
        with source.open("rb") as stream:
            stream.seek(start)
            self.wfile.write(stream.read(count))

    def log_message(self, _format, *_args):
        pass


server = ThreadingHTTPServer(("127.0.0.1", 0), RangeHandler)
server_thread = threading.Thread(target=server.serve_forever, daemon=True)
server_thread.start()
port = server.server_address[1]

disk = BUILD / "virtio-sound-state.img"
wav = BUILD / "virtio-sound-output.wav"
log = BUILD / "virtio-sound-serial.log"
disk.write_bytes(b"")
with disk.open("r+b") as stream:
    stream.truncate(32 * 1024 * 1024)
wav.unlink(missing_ok=True)

command = [
    "qemu-system-x86_64", "-machine", "pc", "-cpu", "max", "-smp", "4",
    "-m", "256M", "-vga", "std", "-display", "none",
    "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0",
    "-audiodev", f"driver=wav,id=audio0,path={wav}",
    "-device", "virtio-sound-pci,audiodev=audio0",
    "-drive", f"file={disk},format=raw,if=ide,index=0",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-boot", "once=d",
    "-cdrom", str(BUILD / "expos.iso"), "-serial", "stdio", "-no-reboot",
]
process = subprocess.Popen(
    command,
    cwd=ROOT,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output = bytearray()
stage = "wait-missing"
deadline = time.monotonic() + 600
try:
    process.stdin.write(b"1111\noperator\nexpos\n~t\nplay imtq\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 65536)
            if chunk:
                output.extend(chunk)
        if b"EXPOS_AYO_MUSIC_DOWNLOAD_ERROR" in output or b"EXPOS_FORM_FAULT" in output:
            raise RuntimeError(f"guest download failed at {stage}")
        if stage == "wait-missing" and b"EXPOS_MEDIA_NOT_INSTALLED track=imtq" in output:
            process.stdin.write(
                f"ayo music install imtq http://10.0.2.2:{port}/IMTQ.mp3\n".encode()
            )
            process.stdin.flush()
            stage = "wait-imtq-install"
        elif stage == "wait-imtq-install" and b"EXPOS_AYO_MUSIC_COMMIT track=imtq" in output:
            process.stdin.write(b"play imtq\n")
            process.stdin.flush()
            stage = "wait-imtq-play"
        elif stage == "wait-imtq-play" and b"EXPOS_MEDIA_PLAY codec=mp3 track=imtq" in output:
            time.sleep(1.5)
            process.stdin.write(
                f"audiostop\nayo music install dash http://10.0.2.2:{port}/dash.mp3\n".encode()
            )
            process.stdin.flush()
            stage = "wait-dash-install"
        elif stage == "wait-dash-install" and b"EXPOS_AYO_MUSIC_COMMIT track=dash" in output:
            process.stdin.write(b"play dash\n")
            process.stdin.flush()
            stage = "wait-dash-play"
        elif stage == "wait-dash-play" and b"EXPOS_MEDIA_PLAY codec=mp3 track=dash" in output:
            time.sleep(1.5)
            process.stdin.write(b"audio\naudiostop\nshutdown\n")
            process.stdin.flush()
            stage = "done"
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError(f"VirtIO Sound Ayo download timed out at {stage}")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    server.shutdown()
    server.server_close()
    log.write_bytes(output)

text = output.decode(errors="replace")
assert stage == "done", f"music workflow stopped at {stage}; see {log}"
assert process.returncode == 33, f"guest exited with {process.returncode}; see {log}"
assert "EXPOS_VIRTIO_SOUND_READY" in text, log
assert "EXPOS_AUDIO_READY backend=virtio-sound" in text, log
assert "EXPOS_MEDIA_NOT_INSTALLED track=imtq" in text, log
assert "EXPOS_AYO_MUSIC_COMMIT track=imtq bytes=7585316" in text, log
assert "EXPOS_AYO_MUSIC_COMMIT track=dash bytes=2256476" in text, log
assert "EXPOS_MEDIA_STORE_ERROR" not in text, log
assert "EXPOS_MEDIA_ERROR" not in text, log
assert "EXPOS_AUDIO_FAULT" not in text, log
statuses = [line for line in text.splitlines() if "EXPOS_AUDIO_STATUS backend=virtio-sound" in line]
assert statuses and "completion=virtqueue" in statuses[-1] and "completed=0" not in statuses[-1]
assert wav.stat().st_size > 44 + 48_000 * 4, "captured less than one second of PCM"

# Reboot with the web server already gone. Both play commands must load the
# committed catalog/data slots rather than silently fetching or using ISO data.
persist_wav = BUILD / "virtio-sound-persist-output.wav"
persist_log = BUILD / "virtio-sound-persist-serial.log"
persist_wav.unlink(missing_ok=True)
persist_command = [
    value.replace(str(wav), str(persist_wav)) if value.startswith("driver=wav,") else value
    for value in command
]
process = subprocess.Popen(
    persist_command,
    cwd=ROOT,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)
selector = selectors.DefaultSelector()
selector.register(process.stdout, selectors.EVENT_READ)
output = bytearray()
stage = "wait-imtq"
deadline = time.monotonic() + 120
try:
    process.stdin.write(b"1111\noperator\nexpos\n~t\nplay imtq\n")
    process.stdin.flush()
    while time.monotonic() < deadline:
        for key, _ in selector.select(0.1):
            chunk = os.read(key.fd, 65536)
            if chunk:
                output.extend(chunk)
        if stage == "wait-imtq" and b"EXPOS_MEDIA_PLAY codec=mp3 track=imtq" in output:
            process.stdin.write(b"audiostop\nplay dash\n")
            process.stdin.flush()
            stage = "wait-dash"
        elif stage == "wait-dash" and b"EXPOS_MEDIA_PLAY codec=mp3 track=dash" in output:
            process.stdin.write(b"audiostop\nshutdown\n")
            process.stdin.flush()
            stage = "done"
        if process.poll() is not None:
            output.extend(process.stdout.read())
            break
    else:
        raise TimeoutError(f"persistent playback timed out at {stage}")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    selector.close()
    persist_log.write_bytes(output)

persist_text = output.decode(errors="replace")
assert stage == "done", persist_log
assert process.returncode == 33, persist_log
assert "EXPOS_MEDIA_NOT_INSTALLED" not in persist_text, persist_log
assert "EXPOS_AYO_MUSIC_COMMIT" not in persist_text, persist_log
assert persist_wav.stat().st_size > 44, persist_wav
