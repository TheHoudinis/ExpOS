# ExpPython and the Python SDK

ExpPython embeds the real MicroPython 1.26.0 interpreter in the x86-64 kernel.
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
Form scripts currently fit the shell's 512-byte content capacity; inline
commands are limited by its 127-byte input line. The embedding accepts up to
4096 source bytes. The runtime limits are a private 256 KiB heap, 48 KiB C stack
budget, 100000 VM/iterator budget checkpoints, and 16 KiB output. There is no
claim that a checkpoint equals a CPU instruction or a fixed number of seconds.
Loop and iterator exhaustion use a VM abort that Python `try/except` cannot
swallow. C iterator loops, including `sum(range(...))`, are also charged.
Exceeding a limit returns to the shell; subsequent scripts get a fresh VM.

This is an in-kernel interpreter without separate address-space isolation.
It exposes no raw memory/device/network primitives to scripts; it is not a
general native application loader or a guarantee against interpreter defects.
The current console font displays ASCII; non-ASCII output uses placeholders.

`sdk/python` is a separate host-side Python development package. Its versioned
request/response serialization matches the Rust and Go ABI, and its bounded
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
