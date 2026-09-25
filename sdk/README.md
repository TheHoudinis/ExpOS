# ExpOS SDK

The SDK targets **Form ABI v1** across Rust, Go, C, and Python. Rust, Go, and C
are implementation SDKs; Python currently supplies both the host SDK and the
bounded ExpPython runtime. ASL remains an architecture target rather than a
finished compiler toolchain.

The shared project driver understands `expos.toml`:

```toml
[package]
name = "TextLab"
version = "2.4.0"
language = "rust"

[form]
fin = "91AF0000-0000-0000-0000-000000000001"
kind = "executable"
capabilities = ["display", "input", "read"]
dependencies = ["ExpDisplay@>=1.0.0"]
architectures = ["x86_64"]
```

Then run:

```sh
./sdk/bin/expos build
./sdk/bin/expos run
./sdk/bin/expos test
./sdk/bin/expos package
```

`package` creates a deterministic tar artifact, a machine-readable Ayo receipt,
and prints the exact `ayo slap` command with the artifact SHA-256. The tool has
fixed Rust, Go, and C build recipes and does not execute arbitrary package
hooks from the manifest.

`run` is explicitly a host-development run today. It does not claim native
ExpOS execution: the kernel still needs its user-mode call gate, address-space
switching, and implementation loader before SDK binaries can run as isolated
native Forms.
