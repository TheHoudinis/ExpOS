import hashlib
import json
from pathlib import Path
import subprocess
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
