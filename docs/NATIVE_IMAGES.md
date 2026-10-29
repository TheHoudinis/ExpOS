# EXF1 executable Forms in ExpOS v9.4 “Lose”

An EXF1 Form contains contributor-produced x86-64 instructions. The kernel
validates the image, copies its segments into an owned native slot, and enters
its entry point at CPL3. It does not translate the image into a built-in capsule.
Admission still requires an active executable FIN, the current CFC/Dimension,
an Execute Handle, ExpSeal and the existing hardware-tick budget.

Persisted executable Forms now have a **32 KiB per-Form quota** inside a
**48 KiB per-CFC live blob budget**. Content is allocated in 4 KiB chunks and
the disk snapshot writes only bytes that exist; it is no longer a 4 KiB array
inside every Form record. It still cannot contain MicroPython or a full network service.
There is no ELF loader, relocation, dynamic linker, language allocator, TLS, libc or floating
point/SIMD support. Container v1 keeps its original one-page layout. Container
v2 admits up to eight RX code pages, six RW/NX data pages and four RW/NX stack
pages, with a private 64 KiB RW/NX heap and unmapped guards around data, heap and stack.
Native images require CPU NX; older CPUs retain
only the diagnostic capsule path. MicroPython and HTTP parsing use separate
built-in Ring 3 ELF services. A third service owns DHCP, DNS, ICMP, generic UDP,
plain TCP/HTTP and authenticated streaming TCP/TLS. It also owns the RTL8139
polling driver and private DMA buffers. Ring 0 supplies only a validated
register/value portal, substitutes pinned DMA addresses and revokes bus
mastering before teardown. Hardware IOMMU confinement is not implemented.

## Build and import

The C toolchain needs an x86-64 GCC-compatible compiler and GNU-compatible
linker (`CC` may select the compiler). Rust uses the installed freestanding
target. Native Go uses ExpOS's bounded source profile and does not link the host
Go runtime or standard library. No external libraries are linked.

```sh
sdk/bin/expos --manifest sdk/examples/native-hello/expos.toml build
sdk/bin/expos --manifest sdk/examples/native-hello/expos.toml import-script --no-build
sdk/bin/expos --manifest sdk/examples/native-hello/expos.toml package --no-build
sdk/bin/expos --manifest sdk/examples/native-go/expos.toml build
make native-image-check
```

Run the emitted import commands in the ExpOS console as an authorized operator.
They open a private Ayo native transaction with the exact byte length and
SHA-256, transfer bounded chunks, validate the complete EXF1 image, atomically
publish `NativeHello`, grant Execute, and execute it. The sample logs `Hello
from compiled C in Ring 3` and exits with result 42 after checking initialized
data, zeroed BSS and writable storage. `expos run` deliberately rejects native
targets because it is the host development launcher.

`ayo native begin <Form> <bytes> <sha256>` never changes the installed Form.
`ayo native write <offset> <hex>` accepts only contiguous bounded chunks into
private staging memory. `ayo native commit` publishes only when the declared
length, digest and full EXF1 validation all pass. An incomplete, malformed or
digest-mismatched replacement is discarded, leaving the preceding executable
and its revision intact. `ayo native abort` explicitly drops a stage. Execute
authority is still granted separately so package bytes cannot grant themselves
capabilities.

`writehex <Form> <offset> <hex>` shares the existing mutation policy, lifecycle,
resource accounting and persistence path. Offset zero replaces content;
nonzero offsets must equal its current length. Invalid hex, offsets or capacity
leave content unchanged. Each accepted chunk commits independently. This
remains a low-level development mutation interface, **not a package
installation path**. SDK-generated native imports now use the Ayo transaction
above; interrupted Ayo staging never replaces the old executable.

`package` marks native metadata `implementation_format: "exf1"`, architecture
`x86_64`, and hashes the actual executable. Host builds are marked `host`.
The host Ayo archive and the kernel's installed-app catalog are still separate.
The kernel can automatically download a same-origin HTTPS manifest/artifact,
verify a caller-pinned Ed25519 key and SHA-256, then use the same transactional
EXF1 installer. A published production registry and release key remain open.
Deterministic tar metadata and two-directory native build equality are tested
with the installed toolchain; cross-toolchain reproducibility is not claimed.

## Container v1 and v2

All integers are unsigned little-endian. The exact file length must equal
`24 + code_bytes + data_bytes`. Unknown versions, architectures, ABI, flags,
reserved bits, trailing bytes, empty code and out-of-bounds entries are rejected
before a native slot is reserved. No executable permissions come from input.

| Offset | Type | Meaning |
|---|---|---|
| 0 | 4 bytes | ASCII `EXF1` |
| 4 | u16 | Container/bootstrap version, 1 or 2 |
| 6 | u16 | Architecture, 1 = x86-64 |
| 8 | u16 | Form ABI version, 1 |
| 10 | u16 | Flags, must be zero |
| 12 | u16 | Code bytes (includes read-only constants) |
| 14 | u16 | Initialized data bytes |
| 16 | u16 | Additional zero-filled bytes, including linker alignment gaps |
| 18 | u16 | Entry offset within code |
| 20 | u32 | Reserved, must be zero |
| 24 | bytes | Code, followed immediately by initialized data |

In v1, code starts at `0x400000000000`. Data starts at `0x400000001000`;
initialized plus zero-filled data must fit 3,840 bytes. The rest of the data
and stack pages is also zeroed on every admission. In v2, code may occupy eight
pages, data starts at `0x400000009000`, and initialized plus zero-filled data
must fit 24 KiB minus the 256-byte ABI prefix. A private 64 KiB RW/NX
per-context heap and four-page stack follow with unmapped guard boundaries. There is no checksum or
signature in EXF1: the format validator establishes structure, not provenance.
Package hashes/signatures must be checked by the installing authority.

## Bootstrap and CPU entry

The integer SysV entry convention is `_start(request, response, execute, log)`.
For v1, RDI = `0x400000001000`; for v2, RDI = `0x400000009000`. RSI is RDI +
`0x50`, EDX is the Execute Handle ID and ECX is the Log Handle ID. The loader initializes the ABI request's version, caller
FIN and Log Handle; the initial Log arguments describe the initialized data.
Page `0x400000002000` is an unmapped guard; the v1 stack top is
`0x400000003ff0`, with function-entry alignment and a zero return sentinel.
`_start` must use `ExecutionExit`, not return. The request is
72 bytes and response 40 bytes; see [Form ABI v1](FORM_ABI_V1.md). The bootstrap
does not change frozen ABI call numbers or request/response layouts.

`sdk/c/expos_native.h` implements `int 0x80` with the exact admitted pointers
and RAX magic. Log, HandleAuthorize, TimeNow, ExecutionYield and ExecutionExit
are connected; other calls return Unsupported. The transport preserves integer
registers across a returning ABI call. Handles remain broker-validated; their
IDs alone cannot grant rights or let a caller substitute another FIN.

Kernel/user GS bases are exchanged on CPL3 transitions. User GS-selector changes
cannot overwrite the per-CPU kernel base used by timer/ABI/fault handling.
FSGSBASE and OSXSAVE are disabled; CR0.TS blocks floating-point state access
before execution and is cleared on kernel entry. Depending on instruction and
CPU setup, unsupported SIMD faults as #UD or #NM. SYSCALL is disabled and
SYSENTER has no entry selector; only the installed interrupt gate is admitted.
This does not add NMI/IST handling. V2 code pages come from the physical-frame
allocator; after loading, their supervisor identity aliases are changed to
read-only/NX while their Form mappings remain read-only/executable. Reclaim
restores RW/NX before the frame returns to the allocator.

## Evidence

`make native-image-check` builds C, Rust and Go through the SDK, imports them into the normal
BIOS image, checks output/result and data/BSS, then reboots the same disposable
state disk and executes the persisted bytes again. Hostile images attempt GS
mutation, floating point/SIMD, WRGSBASE, CLI, SYSCALL/SYSENTER, code writes,
execution from data and an infinite loop. The survivor must still execute.
A separate NX-disabled boot must reject EXF1 while a diagnostic capsule works.
Logs are under `build/native-image/`. No physical disk is used.

Core tests validate header/segment boundaries and every seed truncation.
`make parser-fuzz` includes 100,000 deterministic EXF1 mutations by default.
This is not coverage-guided fuzzing, a sanitizer campaign or a complete audit.


Cliptick compiles `.c`, `.rs`, and `.go` source on-device into EXF1, then runs
the generated image in a fresh Ring 3 address space. Local mode atomically saves
both source and executable Form; sandbox mode keeps both ephemeral. This
compiler deliberately implements the shared freestanding profile: one `main`,
an optional bounded `expos_log`/`expos::log`/`expos.Log` string and an unsigned
exit result. It is not GCC, rustc, or the upstream Go toolchain/runtime and has
no packages, GC, heap, libc, standard library, floating point or general syntax.
Imported host-built EXF1 Forms remain supported. External images still cannot
use the built-in workers' IPC reply channel.
