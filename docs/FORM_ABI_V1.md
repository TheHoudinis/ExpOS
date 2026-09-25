# ExpOS Form ABI v1

Status: **frozen contract**. Kernel releases may add new calls and status codes,
but may not change the meaning, number, alignment, or field width of anything
defined here. An incompatible design requires Form ABI v2 and explicit loader
selection; it must not silently reinterpret v1.

This is a Form-and-capability boundary, not a POSIX syscall layer. A caller is
identified by a FIN and presents a requester-bound Form Handle. Names resolve
to Forms; authority is never inferred from a path, process ID, user ID, file
descriptor, or ambient kernel object.

## Wire records

All integers are unsigned little-endian. Records are naturally aligned to 8
bytes. FIN bytes use their canonical network-order display sequence and are
not byte-swapped as an integer.

```text
Request (72 bytes)
  0  u16 version       must be 1
  2  u16 call
  4  u8  caller_fin[16]
 20  u32 handle
 24  u64 arguments[6]

Response (40 bytes)
  0  u16 status
  2  u8  reserved[6]   zero on send; ignored on receive
  8  u64 values[4]
```

Unknown calls return `UNSUPPORTED`; malformed records return `INVALID`;
missing, revoked, foreign-CFC, expired, wrong-Dimension, or underpowered
Handles return `DENIED`. A bounded service may return `WOULD_BLOCK`. Reserved
fields and unused arguments must be zero so later compatible extensions remain
possible.

## Frozen calls

| Number | Call | Required right | v1 purpose |
|---:|---|---|---|
| 1 | `LOG` | Read | bounded diagnostic record |
| 2 | `FORM_RESOLVE` | Read | resolve a visible Form/FIN binding |
| 3 | `HANDLE_AUTHORIZE` | Execute | check a Handle operation without exercising it |
| 4 | `TIME_NOW` | Read | read an approved monotonic or civil clock |
| 5 | `IPC_SEND` | Execute | send a bounded message through an IPC Form Handle |
| 6 | `IPC_RECEIVE` | Execute | receive a bounded message or `WOULD_BLOCK` |
| 16 | `SURFACE_CREATE` | Display | create an ExpDisplay surface |
| 17 | `BUFFER_ATTACH` | Display | attach a validated Buffer Handle |
| 18 | `SURFACE_DAMAGE` | Display | mark a bounded damaged rectangle |
| 19 | `SURFACE_COMMIT` | Display | atomically publish pending surface state |
| 20 | `EVENT_POLL` | Input | receive a bounded typed event |
| 32 | `BROWSER_NAVIGATE` | Execute | navigate a Browser Interface Form |
| 33 | `STORAGE_READ` | Read | read through a storage/data Form Handle |
| 34 | `STORAGE_WRITE` | Configure | mutate through an ExpFS transaction Handle |
| 35 | `NETWORK_SEND` | Network | submit bytes to a Network Form channel |
| 36 | `NETWORK_RECEIVE` | Network | receive bytes or `WOULD_BLOCK` |
| 48 | `PACKAGE_TRANSACTION` | Package | submit or inspect an Ayo transaction |

Numbers 7–15, 21–31, 37–47, and 49–255 are reserved. Implementations must not
assign private meanings in those ranges.

Calls that exchange bytes use execution-context buffer grants, not raw kernel
pointers. The grant format and native call-gate transport are not frozen yet;
until they land, native buffer-bearing calls must return `UNSUPPORTED`. The
call numbers and record layout are frozen now so SDK source does not depend on
Rust kernel structures while that transport is built.

## Compatibility promise

- v1 request and response sizes stay 72 and 40 bytes.
- Existing call/status numbers and semantics never change within v1.
- New calls are capability-gated and use unused numbers; old kernels return
  `UNSUPPORTED`.
- New response information uses previously unused values or a new call.
- A Handle remains bound to its issuing CFC, requester, target Form, Dimension,
  rights, expiry, and revocation tree.
- SDKs may add convenience methods without changing the wire contract.
- Kernel-private Rust types, memory layouts, and scheduler structures are not
  ABI and must never appear in an SDK.

The Rust, Go, C, and Python contract tests assert the shared call numbers and
record sizes. Native user-mode transport is still pending and is reported as
such; host emulators do not prove a working ring transition or sandbox.
