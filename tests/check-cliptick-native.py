#!/usr/bin/env python3
"""Compile real C/Rust, import through the shell, then execute inside Cliptick."""
import importlib.util
import re
from pathlib import Path
import subprocess
import tempfile
import time
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("editors", ROOT / "tests/check-editors.py")
editors = importlib.util.module_from_spec(spec)
spec.loader.exec_module(editors)
CLI = ROOT / "sdk/bin/expos"
scripts = []
for example in ("native-hello", "native-rust"):
    manifest = ROOT / f"sdk/examples/{example}/expos.toml"
    subprocess.run([str(CLI), "--manifest", str(manifest), "build"], check=True)
    script = subprocess.check_output([str(CLI), "--manifest", str(manifest), "import-script", "--no-build"], text=True)
    scripts.extend(line for line in script.splitlines() if not line.startswith("execute "))

class NativeCheck(editors.EditorCheck):
    def keys(self, *names):
        self.qmp("send-key", keys=[{"type": "qcode", "data": name} for name in names], **{"hold-time": 15})
        time.sleep(0.05)
    def run(self):
        self.wait_for("EXPOS_BOOT_MODE_READY")
        self.connect()
        self.type("2\noperator\nexpos\n")
        self.wait_for("EXPOS_SHELL_READY")
        for command in scripts:
            self.type(command + "\n")
        self.type(
            'mkform NativeHello.c data\n'
            'write NativeHello.c int main(){ expos_log("Hello from compiled C in Ring 3"); return 42; }\n'
        )
        self.type(
            'mkform NativeRust.rs data\n'
            'write NativeRust.rs fn main(){ expos::log("Hello from compiled Rust in Ring 3!"); expos::exit(43); }\n'
        )
        self.type("desktop\n")
        self.wait_for("EXPOS_DESKTOP_EMPTY")
        self.keys("meta_l", "spc")
        self.keys("right")
        self.type("\n")
        self.wait_for("EXPOS_APP_OPENED TERMINAL")
        self.type("ayo install Cliptick\n")
        self.wait_for("operation=1 package=30 name=Cliptick")
        self.type("ayo open Cliptick\n")
        self.wait_for("EXPOS_APP_OPENED AYO APPS")
        self.click(70, 138)
        self.wait_for("EXPOS_EDITOR_OPEN app=Cliptick name=NativeHello.c")
        self.click(250, 74)
        self.wait_for("EXPOS_CLIPTICK_NATIVE_EXIT result=42")
        self.picture("cliptick-native-c")
        self.click(70, 162)
        self.wait_for("EXPOS_EDITOR_OPEN app=Cliptick name=NativeRust.rs")
        self.click(250, 74)
        self.wait_for("EXPOS_CLIPTICK_NATIVE_EXIT result=43")
        self.picture("cliptick-native-rust")
        self.keys("ctrl", "shift", "esc")
        self.type("y")
        self.wait_for("EXPOS_COMMAND_OK desktop")
        self.type("shutdown\n")
        self.wait_for("EXPOS_COMMAND_OK shutdown")
        assert self.process.wait(timeout=5) == 33
        log = self.serial.read_text()
        assert "EXPOS_KERNEL_FAULT" not in log and "[KERNEL PANIC]" not in log
        assert "EXPOS_CLIPTICK_NATIVE_ERROR" not in log
        assert re.search(r"EXPOS_CLIPTICK_NATIVE_EXIT result=42 ticks=\d+ bytes=31", log)
        assert re.search(r"EXPOS_CLIPTICK_NATIVE_EXIT result=43 ticks=\d+ bytes=35", log)
        print("CLIPTICK NATIVE: real imported C and Rust executed in Ring 3; captured output and results 42/43")

with tempfile.TemporaryDirectory(prefix="expos-native-ide-") as directory:
    test = NativeCheck("bios", Path(directory), time.monotonic() + 180, ROOT / "build/cliptick-native-check")
    try:
        test.run()
    finally:
        if test.process.poll() is None and test.wire is not None:
            try:
                test.qmp("screendump", filename=str(test.artifacts / "last.ppm"))
            except Exception:
                pass
        test.close()
