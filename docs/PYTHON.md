# ExpPython and the Python SDK

ExpPython runs the real MicroPython 1.26.0 interpreter in an isolated x86-64 Ring 3 service.
It supports Python functions, classes, comprehensions, generators, exceptions,
integers, strings, lists, tuples, dictionaries and sets. It is not CPython;
this small build has no floating point, arbitrary-precision integers, pip,
native extensions, filesystem/network access, or general standard library.
Its upstream source and MIT attribution are retained under `vendor/micropython`.

From the console:

```text
python print(sum(x*x for x in range(5)))
mkform Script data
write Script print('hello from a Form')
python -f Script
python exec('def square(x):\n return x*x\nprint(square(7))')
```

`python3` is an alias. Every execution requires the authenticated session's
Execute capability, including requester identity and revocation checks.
Each invocation starts a fresh VM: variables do not survive between commands.
Form scripts fit the shell's 4096-byte content capacity; inline
commands are limited by its 127-byte input line. The embedding accepts up to
4096 source bytes. The runtime limits are a private 256 KiB heap, 48 KiB C stack
budget, 100000 VM/iterator budget checkpoints, and 16 KiB output. There is no
claim that a checkpoint equals a CPU instruction or a fixed number of seconds.
Loop and iterator exhaustion use a VM abort that Python `try/except` cannot
swallow. C iterator loops, including `sum(range(...))`, are also charged.
Exceeding a limit returns to the shell; subsequent scripts get a fresh VM.
Genesis Minimal and Essentials installations omit this runtime. Everything
enables it, while Custom can explicitly include or omit it. Paranoid always
disables it and cannot re-enable it during the boot.

Each invocation owns a private address space with W^X image mappings, read-only
source input, a 128 KiB mapped stack and sealed Execute/reply Handles. The 48 KiB
VM stack budget remains inside that mapping. Faults and a 2000-tick hardware
watchdog stop the worker, release its arena and permit another VM invocation.
NX is required; unsupported CPUs refuse execution. No raw memory, device,
filesystem or network service is exposed to scripts. The loader, ABI bridge
and scheduler remain trusted code; this is not a guarantee against all defects.

The optional Ayo app Cliptick uses this same native VM and captures output in
its Output pane. Ctrl+R runs and Ctrl+S saves. Its source documents fit 4096
bytes. See [Loop work log](LOOP_V9_3.md) for run-mode semantics.
The current console font displays ASCII; non-ASCII output uses placeholders.

`sdk/python` is a separate host-side Python development package. Its versioned
request/response serialization matches the language-neutral Form ABI used by
the Rust, Go, and C SDKs, and its bounded
emulator verifies capability identity, revocation and atomic surface commits.
Unsupported services return explicit errors. A live transport to a running
kernel is not yet implemented, and this host SDK is not imported by ExpPython.

```sh
make python-sdk            # host SDK contract tests
make python-runtime-check  # real interpreter + exhaustion/recovery tests
make python-check          # run Python and capability denial inside QEMU
```

The runtime build is offline after checkout and uses GCC, Make, Python 3, and
the pinned vendored sources. See the upstream [porting documentation](https://docs.micropython.org/en/v1.26.0/develop/porting.html)
and [local source provenance](../vendor/micropython/EXPOS_PORT.md).
