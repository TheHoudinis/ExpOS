#!/usr/bin/env python3
"""Real PS/2 editor input, Ayo installation, isolated Python and scanout."""
import importlib.util
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread
from pathlib import Path
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("startup", ROOT / "tests/check-startup.py")
startup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(startup)

class Pages(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        if self.path == "/large":
            body = b"<title>Large page</title><a href='/'>Back to link test</a>" + b"<p>Readable page content</p>" * 70
            content_type = "text/html"
        elif self.path == "/full":
            body = (
                b"<link rel='stylesheet' href='/style.css'><title>Full page</title>"
                b"<a href='/large'>Load complete site</a><h1>Hydrated page</h1>"
                b"<p id='status'>Waiting for active content</p>"
                + b"<p class='card'>Material content card</p>" * 24
                + b"<script src='/script.js'></script>"
            )
            content_type = "text/html"
        elif self.path == "/style.css":
            body = b"h1{color:#a8d9df}.card{background:#182427;border-radius:12px;padding:8px}"
            content_type = "text/css"
        elif self.path == "/script.js":
            body = (
                b"localStorage.setItem('mode','full');document.cookie='sid=full';"
                b"document.getElementById('status').textContent=localStorage.getItem('mode');"
                b"fetchText('/data','status');"
            )
            content_type = "text/javascript"
        elif self.path == "/data":
            body = b"Fully hydrated without nesting the input frame"
            content_type = "text/plain"
        else:
            body = b"<title>Link test</title><a href='/full'>Open full page</a>"
            content_type = "text/html"
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

class EditorCheck(startup.Startup):
    def type(self, text):
        plain = {"\n": "ret", " ": "spc", ".": "dot", "'": "apostrophe", "-": "minus", "/": "slash", "=": "equal", ",": "comma", ";": "semicolon"}
        shifted = {"(": "9", ")": "0", ":": "semicolon", "_": "minus", "+": "equal", "*": "8", "!": "1", "{": "bracket_left", "}": "bracket_right", '"': "apostrophe"}
        for char in text:
            if char in shifted:
                self.keys("shift", shifted[char])
            elif char.isupper():
                self.keys("shift", char.lower())
            else:
                self.keys(plain.get(char, char))

    def terminal(self):
        # Cycling from Apps selects a previously visible Terminal.
        self.keys("meta_l", "tab")
        self.wait_for("EXPOS_APP_OPENED TERMINAL")

    def click(self, x, y):
        def move(dx, dy):
            self.qmp("input-send-event", events=[
                {"type": "rel", "data": {"axis": "x", "value": dx}},
                {"type": "rel", "data": {"axis": "y", "value": dy}},
            ])
            time.sleep(0.035)
        for _ in range(12):
            move(-100, -100)
        while x or y:
            dx, dy = min(100, x), min(100, y)
            move(dx, dy)
            x -= dx; y -= dy
        for down in (True, False):
            self.qmp("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": "left"}}])
            time.sleep(0.15)

    def run(self):
        self.wait_for("EXPOS_BOOT_MODE_READY")
        self.connect()
        self.type("1\n")
        self.wait_for("EXPOS_LOGIN_SCREEN_PRESENTED")
        self.type("operator\nexpos\n")
        self.wait_for("EXPOS_DESKTOP_EMPTY")
        self.keys("meta_l", "spc")
        self.keys("right")
        self.type("\n")
        self.wait_for("EXPOS_APP_OPENED TERMINAL")
        self.type("ayo install Cliptick\n")
        self.wait_for("operation=1 package=30 name=Cliptick")
        self.type("ayo install Writer\n")
        self.wait_for("operation=1 package=31 name=Writer")
        self.type("ayo open Cliptick\n")
        self.wait_for("EXPOS_APP_OPENED AYO APPS")
        self.type("print(6*7)")
        self.keys("ctrl", "r")
        self.wait_for("EXPOS_CLIPTICK_EXIT result=0")
        self.keys("ctrl", "s")
        self.wait_for("EXPOS_EDITOR_SAVE app=Cliptick name=main.py")
        self.picture("cliptick-python")
        self.click(334, 74)
        self.click(250, 140)
        self.click(250, 182)
        self.click(250, 224)
        self.picture("cliptick-settings")
        self.click(334, 74)
        self.keys("ctrl", "r")
        self.wait_for("EXPOS_CLIPTICK_RUN language=python location=local authority=operator")
        self.wait_for("EXPOS_CLIPTICK_EXIT result=0", count=2)
        self.picture("cliptick-local")
        self.click(46, 74)
        self.click(250, 110)
        for _ in range(len("main.py")):
            self.keys("backspace")
        self.type("main.go\n")
        self.click(260, 140)
        self.type('package main\nfunc main() { expos.Log("hello Go") expos.Exit(43) }')
        self.keys("ctrl", "r")
        self.wait_for("EXPOS_CLIPTICK_COMPILE language=Go storage=expfs")
        self.wait_for("EXPOS_CLIPTICK_NATIVE_EXIT result=43")
        self.picture("cliptick-native-go")
        self.terminal()
        self.type("ayo open Writer\n")
        self.type("A small plain text document.")
        self.keys("left")
        self.keys("left")
        self.type("!")
        self.keys("ctrl", "s")
        self.wait_for("EXPOS_EDITOR_SAVE app=Writer name=Untitled.txt")
        self.picture("writer")
        self.click(70, 138)
        self.wait_for("EXPOS_EDITOR_OPEN app=Writer name=main.py bytes=10")
        self.picture("writer-python")
        self.terminal()
        self.type("style prompt compact\n")
        self.type("style cursor beam\n")
        self.type("style color amber\n")
        self.type("trace services\n")
        # A deliberately black terminal is not the startup screen's lit-area test.
        self.qmp("screendump", filename=str(self.artifacts / "terminal-diagnostics.ppm"))
        width, height, pixels = startup.read_ppm(self.artifacts / "terminal-diagnostics.ppm")
        assert (width, height) == (640, 480) and len(set(pixels)) >= 8
        self.type("open browser\n")
        self.wait_for("EXPOS_APP_OPENED BROWSER")
        self.type(f"/http://10.0.2.2:{server.server_port}/\n")
        self.wait_for("EXPOS_BROWSER_HTTP_OK status=200", count=1)
        for count in range(2, 7):
            self.click(70, 153)
            self.wait_for("EXPOS_BROWSER_HTTP_OK status=200", count=count)
        self.wait_for("EXPOS_BROWSER_NAVIGATION_DEQUEUED", count=6)
        self.wait_for("EXPOS_BROWSER_RESOURCES discovered=2 loaded=2 rejected=0", count=2)
        self.wait_for("EXPOS_BROWSER_READER reason=NodeLimit", count=2)
        self.picture("browser-link-survivor")
        self.keys("ctrl", "shift", "esc")
        self.type("y")
        self.wait_for("EXPOS_SHELL_READY")
        self.type("serviceinfo\n")
        self.type("bootlog CLIPTICK\n")
        self.type("shutdown\n")
        self.wait_for("EXPOS_COMMAND_OK shutdown")
        assert self.process.wait(timeout=5) == 33
        log = self.serial.read_text()
        assert "EXPOS_KERNEL_FAULT" not in log and "[KERNEL PANIC]" not in log
        assert "EXPOS_SERVICE_EXIT" in log and "ring=3" in log
        print("EDITORS: Ayo install, PS/2 editing, persisted source, Ring 3 Python and on-device native Go, queued full-page link hydration and scanout passed")

class RebootCheck(EditorCheck):
    def run(self):
        self.wait_for("EXPOS_BOOT_MODE_READY")
        self.connect()
        self.type("1\n")
        self.wait_for("EXPOS_LOGIN_SCREEN_PRESENTED")
        self.type("operator\nexpos\n")
        self.wait_for("EXPOS_DESKTOP_EMPTY")
        self.keys("meta_l", "spc")
        self.keys("right")
        self.type("\n")
        self.wait_for("EXPOS_APP_OPENED TERMINAL")
        self.type("ayo open Cliptick\n")
        self.wait_for("EXPOS_APP_OPENED AYO APPS")
        self.click(70, 138)
        self.wait_for("EXPOS_EDITOR_OPEN app=Cliptick name=main.py bytes=10")
        self.keys("ctrl", "r")
        self.wait_for("EXPOS_CLIPTICK_RUN language=python location=local authority=normal")
        self.wait_for("EXPOS_CLIPTICK_EXIT result=0")
        self.picture("persisted-cliptick")
        self.terminal()
        self.type("shutdown\n")
        self.wait_for("EXPOS_COMMAND_OK shutdown")
        assert self.process.wait(timeout=5) == 33
        assert "EXPOS_KERNEL_FAULT" not in self.serial.read_text()
        print("EDITORS REBOOT: installed catalog, Python source and local/theme settings persisted; Operator intent cleared")

def main():
    global server
    server = ThreadingHTTPServer(("127.0.0.1", 0), Pages)
    Thread(target=server.serve_forever, daemon=True).start()
    editor_timeout = float(os.environ.get("EXPOS_EDITORS_TIMEOUT", "240"))
    reboot_timeout = float(os.environ.get("EXPOS_EDITORS_REBOOT_TIMEOUT", "150"))
    with tempfile.TemporaryDirectory(prefix="expos-editors-") as directory:
        test = EditorCheck(
            "bios",
            Path(directory),
            time.monotonic() + editor_timeout,
            ROOT / "build/editor-check",
        )
        try:
            test.run()
        finally:
            if test.process.poll() is None and test.wire is not None:
                try:
                    test.qmp("screendump", filename=str(test.artifacts / "last.ppm"))
                except Exception:
                    pass
            test.close()
        test = RebootCheck(
            "bios",
            Path(directory),
            time.monotonic() + reboot_timeout,
            ROOT / "build/editor-reboot-check",
        )
        try:
            test.run()
        finally:
            test.close()
    server.shutdown()
    server.server_close()

if __name__ == "__main__":
    main()
