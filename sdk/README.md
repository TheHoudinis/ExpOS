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

`run` launches host development builds. For a real integer-only CPL3 executable,
set `[build] target = "expos-x86_64"` in a C, freestanding Rust, or bounded
native Go project. New C and Rust builds use EXF1 v2 with up to eight code
pages, six data pages and a 32 KiB persisted Form quota. Existing EXF1 v1
artifacts remain accepted unchanged; bounded native Go currently emits v1.

```sh
./sdk/bin/expos --manifest sdk/examples/native-hello/expos.toml build
./sdk/bin/expos --manifest sdk/examples/native-hello/expos.toml import-script --no-build
```

Run the emitted commands in the ExpOS console. The sample logs from Ring 3 and
exits with result 42. `package` preserves the executable bytes and declares
`implementation_format: "exf1"`; automatic native Ayo installation is still
pending. See [native image format, bootstrap and limitations](../docs/NATIVE_IMAGES.md).


Freestanding Rust is also supported (integer-only, `no_std`, `no_main`, no heap
or standard library). `build.entry` names a single Rust source file; the SDK
sets `EXPOS_SDK` for including `sdk/rust/native.rs`. The sample uses that exact
ABI-page transport and exits with result 43:

```sh
./sdk/bin/expos --manifest sdk/examples/native-rust/expos.toml build
./sdk/bin/expos --manifest sdk/examples/native-rust/expos.toml import-script --no-build
```

Cliptick can run an imported executable from a `.c`, `.rs` or `.go` source tab
and can compile its bounded freestanding source profile directly on-device.
Local mode saves the source plus generated executable Form; sandbox mode does
not persist either. Python runs source directly. The on-device profile is not a
replacement for GCC, rustc, or upstream Go: it supports one entry point,
bounded logging and an integer exit result. Host C/Rust builds retain their
full freestanding compilers, while host native Go intentionally emits the same
small runtime-free profile as Cliptick.

```sh
./sdk/bin/expos --manifest sdk/examples/native-go/expos.toml build
./sdk/bin/expos --manifest sdk/examples/native-go/expos.toml import-script --no-build
```
