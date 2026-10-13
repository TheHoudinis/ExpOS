# EXF1 executable Forms (Loop development)

An EXF1 Form contains contributor-produced x86-64 instructions. The kernel
validates the image, copies its segments into an owned native slot, and enters
its entry point at CPL3. It does not translate the image into a built-in capsule.
Admission still requires an active executable FIN, the current CFC/Dimension,
an Execute Handle, ExpSeal and the existing hardware-tick budget.

This first format fits the current persisted Form content limit of **4,096 bytes**,
including the header. It cannot contain MicroPython or a full network service.
There is no ELF loader, relocation, dynamic linker, heap, TLS, libc or floating
point/SIMD support. Integer-only code has one RX code page, one RW/NX ABI/data
page and one RW/NX stack page. Native images require CPU NX; older CPUs retain
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
They create `NativeHello`, transfer the binary in bounded hex chunks, grant
Execute, and execute it. The sample logs `Hello from compiled C in Ring 3` and
exits with result 42 after checking initialized data, zeroed BSS and writable
storage. Use a new Form name for new projects. `expos run` deliberately rejects
native targets because it is the host development launcher.

`writehex <Form> <offset> <hex>` shares the existing mutation policy, lifecycle,
resource accounting and persistence path. Offset zero replaces content;
nonzero offsets must equal its current length. Invalid hex, offsets or capacity
leave content unchanged. Each accepted chunk commits independently. This is a
development import interface, **not a transactional package installation**:
interrupted imports may leave a partial image, which cannot pass image length
validation. An interrupted replacement does not preserve the old executable.

`package` marks native metadata `implementation_format: "exf1"`, architecture
`x86_64`, and hashes the actual executable. Host builds are marked `host`.
The host Ayo archive and the kernel's installed-app catalog are still separate;
automatic native download/install and a published signed registry remain open.
Deterministic tar metadata and two-directory native build equality are tested
with the installed toolchain; cross-toolchain reproducibility is not claimed.

## Container v1

All integers are unsigned little-endian. The exact file length must equal
`24 + code_bytes + data_bytes`. Unknown versions, architectures, ABI, flags,
reserved bits, trailing bytes, empty code and out-of-bounds entries are rejected
before a native slot is reserved. No executable permissions come from input.

| Offset | Type | Meaning |
|---|---|---|
| 0 | 4 bytes | ASCII `EXF1` |
| 4 | u16 | Container/bootstrap version, 1 |
| 6 | u16 | Architecture, 1 = x86-64 |
| 8 | u16 | Form ABI version, 1 |
| 10 | u16 | Flags, must be zero |
| 12 | u16 | Code bytes (includes read-only constants) |
| 14 | u16 | Initialized data bytes |
| 16 | u16 | Additional zero-filled bytes, including linker alignment gaps |
| 18 | u16 | Entry offset within code |
| 20 | u32 | Reserved, must be zero |
| 24 | bytes | Code, followed immediately by initialized data |

Code starts at `0x400000000000`. Data starts at `0x400000001100`;
initialized plus zero-filled data must fit 3,840 bytes. The rest of the data
and stack pages is also zeroed on every admission. There is no checksum or
signature in EXF1: the format validator establishes structure, not provenance.
Package hashes/signatures must be checked by the installing authority.

## Bootstrap and CPU entry

The integer SysV entry convention is `_start(request, response, execute, log)`:
RDI = `0x400000001000`, RSI = `0x400000001050`, EDX = Execute Handle ID and
ECX = Log Handle ID. The loader initializes the ABI request's version, caller
FIN and Log Handle; the initial Log arguments describe the initialized data.
The stack starts at `0x400000002ff8`, with function-entry alignment and a zero
return sentinel. `_start` must use `ExecutionExit`, not return. The request is
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
This does not add NMI/IST handling, guard pages or physical-page sealing of
the kernel's writable backing alias for dynamic code.

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
