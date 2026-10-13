import hashlib
import json
from pathlib import Path
import subprocess
import shutil
import struct
import sys
import tarfile
import tempfile
import unittest


CLI = Path(__file__).resolve().parents[2] / "bin" / "expos"


class SDKCLI(unittest.TestCase):
    def project(self, root: Path, name: str = "HelloForm"):
        (root / "build").mkdir()
        artifact = root / "build" / "hello"
        artifact.write_bytes(b"#!/bin/sh\necho hello\n")
        artifact.chmod(0o755)
        (root / "expos.toml").write_text(
            f'''[package]
name = "{name}"
version = "1.2.3"
language = "c"

[form]
kind = "executable"
capabilities = ["display"]
dependencies = ["ExpDisplay@>=1.0.0"]
architectures = ["ASL", "x86_64"]

[build]
entry = "hello.c"
artifact = "build/hello"
''',
            encoding="utf-8",
        )
        return artifact

    def test_native_rust_is_reproducible_and_importable(self):
        sample = CLI.parents[1] / "examples/native-rust"
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            outputs = []
            for directory in (first, second):
                root = Path(directory)
                for name in ("expos.toml", "main.rs"):
                    shutil.copyfile(sample / name, root / name)
                subprocess.run([str(CLI), "--manifest", str(root / "expos.toml"), "build"], check=True, capture_output=True)
                payload = (root / ".expos/build/NativeRust").read_bytes()
                self.assertTrue(payload.startswith(b"EXF1"))
                self.assertLessEqual(len(payload), 4096)
                script = subprocess.check_output([str(CLI), "--manifest", str(root / "expos.toml"), "import-script", "--no-build"], text=True)
                self.assertIn("mkform NativeRust executable", script)
                self.assertTrue(all(len(line) < 128 for line in script.splitlines()))
                outputs.append(payload)
            self.assertEqual(*outputs)

    def test_package_is_deterministic_and_ayo_ready(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.project(root)
            command = [sys.executable, str(CLI), "--manifest", str(root / "expos.toml"), "package", "--no-build"]
            first = subprocess.run(command, check=True, text=True, capture_output=True)
            archive = root / "dist" / "HelloForm-1.2.3.tar"
            first_digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            second = subprocess.run(command, check=True, text=True, capture_output=True)
            self.assertEqual(first_digest, hashlib.sha256(archive.read_bytes()).hexdigest())
            self.assertIn("ayo slap --source", first.stdout)
            self.assertEqual(first.stdout, second.stdout)
            with tarfile.open(archive) as package:
                self.assertEqual(
                    package.getnames(),
                    ["forms/HelloForm/form.json", "forms/HelloForm/implementation"],
                )
                metadata = json.load(package.extractfile("forms/HelloForm/form.json"))
                self.assertEqual(metadata["form_abi"], 1)
                self.assertEqual(metadata["implementation_sha256"], hashlib.sha256(artifact.read_bytes()).hexdigest())

    def native_project(self, root):
        source = CLI.parents[1] / "examples/native-hello"
        for name in ("expos.toml", "hello.c"):
            shutil.copyfile(source / name, root / name)
        return [sys.executable, str(CLI), "--manifest", str(root / "expos.toml")]

    def test_native_build_reproducible_import_and_package(self):
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            images = []
            for temporary in (first, second):
                root = Path(temporary)
                command = self.native_project(root)
                subprocess.run([*command, "build"], check=True, capture_output=True)
                artifact = root / ".expos/build/NativeHello"
                payload = artifact.read_bytes()
                images.append(payload)
                self.assertLessEqual(len(payload), 4096)
                self.assertEqual(payload[:4], b"EXF1")
                code, data, bss, entry = struct.unpack_from("<4H", payload, 12)
                self.assertEqual(len(payload), 24 + code + data)
                self.assertGreater(bss, 0)
                self.assertLess(entry, code)
                script = subprocess.check_output([*command, "import-script", "--no-build"], text=True)
                lines = script.splitlines()
                self.assertTrue(all(len(line) < 128 for line in lines))
                chunks = [line.split() for line in lines if line.startswith("writehex ")]
                self.assertEqual(b"".join(bytes.fromhex(line[3]) for line in chunks), payload)
                self.assertEqual([int(line[2]) for line in chunks], list(range(0, len(payload), 32)))
                subprocess.run([*command, "package", "--no-build"], check=True, capture_output=True)
                with tarfile.open(root / "dist/NativeHello-0.1.0.tar") as package:
                    metadata = json.load(package.extractfile("forms/NativeHello/form.json"))
                    self.assertEqual(metadata["implementation_format"], "exf1")
                    self.assertEqual(metadata["architectures"], ["x86_64"])
                result = subprocess.run([*command, "run"], capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("native Forms run in ExpOS", result.stderr)
                artifact.write_bytes(payload[:24])
                result = subprocess.run([*command, "package", "--no-build"], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
            self.assertEqual(*images)

    def test_native_rejects_storage_overflow_and_undefined_symbols(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            command = self.native_project(root)
            for source in (
                '__attribute__((used)) char huge[4097] = {1}; void _start(void) { for (;;) {} }',
                'extern void missing(void); void _start(void) { missing(); }',
            ):
                (root / "hello.c").write_text(source)
                result = subprocess.run([*command, "build"], capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)

    def test_native_go_profile_is_reproducible_and_importable(self):
        sample = CLI.parents[1] / "examples/native-go"
        images = []
        for _ in range(2):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ("expos.toml", "main.go"):
                    shutil.copyfile(sample / name, root / name)
                command = [sys.executable, str(CLI), "--manifest", str(root / "expos.toml")]
                subprocess.run([*command, "build"], check=True, capture_output=True)
                payload = (root / ".expos/build/NativeGo").read_bytes()
                self.assertLessEqual(len(payload), 4096)
                code, data, _, entry = struct.unpack_from("<4H", payload, 12)
                self.assertEqual(entry, 0)
                self.assertEqual(
                    payload[24 + code:24 + code + data],
                    b"Hello from native Go in Ring 3\n",
                )
                script = subprocess.check_output(
                    [*command, "import-script", "--no-build"], text=True
                )
                self.assertIn("mkform NativeGo executable", script)
                images.append(payload)
        self.assertEqual(*images)

    def test_rejects_unsafe_package_name(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.project(root, "../escape")
            result = subprocess.run(
                [sys.executable, str(CLI), "--manifest", str(root / "expos.toml"), "package", "--no-build"],
                text=True,
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("package.name", result.stderr)


if __name__ == "__main__":
    unittest.main()
