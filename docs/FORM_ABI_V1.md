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
| 7 | `EXECUTION_YIELD` | Execute | end the current hardware-timed slice voluntarily |
| 8 | `EXECUTION_EXIT` | Execute | terminate the calling Form with `arguments[0]` as its result |
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

Numbers 9–15, 21–31, 37–47, and 49–255 are reserved. Implementations must not
assign private meanings in those ranges.

ExpDisplay Portal v3 is an additive service-protocol revision, not a Form ABI
revision. Its feature negotiation, event coalescing/recovery and presentation
diagnostics remain behind the same Display/Input Handles and the frozen v1 call
numbers above; unsupported optional behavior fails without changing request or
response layouts.

The native call gate consumes this frozen request directly and validates CFC,
requester, target, Dimension, expiry, revocation and the operation derived from
the call number before dispatch. Ayo's `PACKAGE_TRANSACTION` is the first live
consumer. Native x86_64 executable Forms use a fixed shared ABI page at a
loader-owned user virtual address and enter an interrupt gate at vector `0x80`.
The kernel accepts only that exact request/response location. `LOG` receives a
bounded `(offset, length)` grant into the same context-owned page; arbitrary
user pointers are never accepted. Buffer-bearing calls whose grant format is
not yet defined return `UNSUPPORTED`.

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
record sizes. `make ring3-check` is the native proof: it observes CPL3 ABI
traffic, distinct CR3 roots, timer preemption, and forced ExpBudget exhaustion
for a Form that never yields. Host emulators remain contract tests only.

## Loop executable bootstrap

The separate [EXF1 format](NATIVE_IMAGES.md) defines entry registers, fixed
segment addresses and the initial Handle/FIN request. This does not change
Form ABI v1 wire records or reserve any new call numbers. The C native transport
and sample are in `sdk/c/expos_native.h` and `sdk/examples/native-hello/`.


## Built-in service reply transport (Loop)

The isolated Python/HTTP workers use call 5 (`IPC_SEND`) with a sealed reply
Handle. `arguments[0]` is a byte offset in the fixed ABI data page, at least 256;
`arguments[1]` is a length at most 512; the range must remain within that 4096-byte
page. Valid bytes append to the current invocation's capture (at most 32768
bytes). An invalid range or exhausted capture returns a non-OK status. Neither
field is an arbitrary user pointer. The broker validates the requester and
Execute right before the copy. `IPC_RECEIVE` is still unsupported. External
capsules/EXF1 images do not receive a service reply capture and call 5 remains
unsupported for those contexts. This is an internal built-in service protocol,
not a new ABI version or general message queue implementation.

## Additive blob streaming contract v2

Form ABI v1 remains frozen at its original 72-byte request, 40-byte response
and call numbers. Large Form content uses a separate `StreamRequest` with
version 2 and a distinct `EXPSTRM2` gate magic. Its calls are `OPEN_READ`,
`OPEN_WRITE`, `READ_CHUNK`, `WRITE_CHUNK`, `COMMIT` and `ABORT`; none consumes
or reinterprets a v1 call number. ExpFS allocates 4 KiB storage chunks; each
gate transfer is bounded to the 3,840-byte window remaining in the admitted ABI
page after its request/response prefix.

Streams are bound to the caller's CFC, target Form FIN and capability Handle.
Writes accumulate in a private chunked blob and become visible only after a
revision check, quota preflight and normal transactional ExpFS snapshot commit.
`ABORT`, stale revisions, quota failure and malformed ranges publish nothing.
Read and write Handles remain distinct through `READ` and `CONFIGURE` rights.

General asynchronous IPC is scheduler-owned: each context has a bounded
message queue, correlation value and optional blob stream ID. Delivery wakes a
waiting context without making the sender run it synchronously. The v1
`IPC_RECEIVE` wire call remains unsupported rather than being silently changed;
new SDK transport for the general queue must use a separately versioned gate.

Built-in ELF64 services have a private region starting at `0x400000200000`.
The loader accepts only checked, non-overlapping page-aligned R, RX or RW
segments, requires an RX entry point, and supplies RO/NX input at offset
`0x180000` and a RW/NX stack at `0x1c0000`. R8/R9 carry the input pointer/length;
RDI/RSI carry the fixed ABI request/response addresses and RDX/RCX carry Execute
and reply Handle IDs. Each invocation is freshly allocated and discarded.
These trusted build artifacts are separate from persisted EXF1 Forms.

## Built-in raw network portal (Loop)

Calls 35 and 36 now have one kernel transport for a sealed built-in network
worker. The seventh SysV entry argument supplies its Network Handle (R10 also
contains it for the fixed service bootstrap); applications, EXF1 contributors,
MicroPython and the HTTP parser are never issued that Handle.

For the sealed worker only, `NETWORK_RECEIVE` with `arguments[2] = 1` returns
the MAC, IPv4 address/netmask, gateway, DNS server and link state in four packed
ABI values. No packet buffer is copied in this mode. This is interface metadata,
not ambient access to the kernel network stack.

The same sealed worker uses `NETWORK_RECEIVE` mode 2 once per invocation to
pin one service-private, writable/NX, contiguous RTL8139 DMA aperture. Ring 0
validates the mapped pages, sizes, alignment and below-4-GiB range. Mode 3 reads
only the MAC, command/link/interrupt status and four transmit-status registers.
`NETWORK_SEND` mode 3 writes only the audited initialization, RX-consumer and
TX-submit registers with exact widths and bounded values. RBSTART/TSAD writes
must request zero: the kernel substitutes addresses from the validated private
aperture, so the worker never supplies a bus address. Bus mastering is enabled
only for the active lease and is disabled before its arena can be freed. The
former raw-frame copy modes are unsupported in production.

DNS lookup now crosses this boundary: the worker implements bounded ARP,
IPv4/UDP construction and DNS A parsing and returns only a four-byte address.
DHCP Discover/Offer/Request/Ack also crosses this boundary and returns one
validated 24-byte address/netmask/gateway/DNS/server/lease record. The
bounded transport operations pass a peer, port, fresh connection seed and at
most 768 HTTP request bytes to the worker. Plain TCP performs ARP, handshake,
ordered receive, acknowledgements, retransmission and close. HTTPS additionally
receives immutable trusted RTC time and a hostname, then performs TLS 1.3 SNI,
certificate-chain/validity/signature verification and record protection before
returning decrypted HTTP bytes. Both return a truncation flag and at most 60 KiB
of wire data in 512-byte internal reply chunks; the decoded HTTP body is capped
at 56 KiB. This is a sealed service boundary, not a change to the frozen 3840-byte
Form ABI v1 payload window. Generic UDP and ICMP echo use dedicated operations
with checksum and peer/sequence validation. The production kernel no longer
constructs or parses DHCP, DNS, ICMP, UDP, TCP or TLS packets. The remaining
network isolation boundary is the trusted PCI/page-validation and register
portal. The polling driver and DMA buffers execute in Ring 3, but IOMMU
isolation is not implemented; a malicious or faulty DMA-capable device can
therefore escape the software-programmed aperture.
