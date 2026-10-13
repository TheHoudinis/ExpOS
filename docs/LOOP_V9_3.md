# v9.3 “Loop” implementation plan

Baseline: v9.2 “Mole”, commit `7121551`. The development branch is
`codex/v9.3-loop`; no release, publication or physical installation is implied.
Preserve FIN, Form Handles, Dimensions, CFC ownership, ExpScope, ExpSeal and
ExpBudget throughout. Priority follows the requested sections 2–6.

## 2. Stronger kernel isolation — critical

First implemented increment:

- Page-aligned linker text and rodata boundaries, separate ELF segment flags,
  supervisor RX kernel text, R/NX rodata and RW/NX data/BSS/heap/stack/MMIO on
  NX-capable CPUs. Split only necessary 2 MiB mappings into 4 KiB pages; reserve
  a bounded 64-table pool and fail if it is exhausted. Unmap page zero.
- CPU-detected SMAP alongside existing WP/NXE/SMEP on every CPU. Clear AC on
  interrupt/exception entry even if a Form set it. Keep the fixed ABI's data
  copies on validated supervisor aliases instead of opening STAC windows.
- AP startup uses a separate writable mailbox and an RX trampoline after
  copying; NXE is set before paging on APs.
- Normalize Form exception frames, check saved CPL and report error/RIP/CR2.
  Contain divide/debug/breakpoint/overflow/bounds/invalid-opcode/device/segment/
  page/x87/alignment/SIMD faults. Supervisor faults halt with a direct serial
  diagnostic even when a Form is active; they cannot reuse its return context.
- Separate QEMU fault-injection image and host parser mutation campaigns.

Second increment: [EXF1 executable loading and the native C SDK](NATIVE_IMAGES.md),
with validated segments, a versioned bootstrap, capability-controlled binary
import, real contributor code in CPL3 and reboot persistence. Harden GS state,
disable unowned fast syscall entries and reject unsupported floating-point
state access. This initial format fits the existing 512-byte Form content limit.

Third increment: owned built-in ELF services, bounded reply capture, isolated
MicroPython and HTTP parsing, hardware watchdog/fault recovery, and Cliptick /
Writer are implemented below. Native C and Rust can execute from imported EXF1
Forms with captured output. At that increment TCP/TLS/DNS and device drivers
remained in the kernel.

Fourth increment: ExpFS v3 expands complete Form records from 512 to 4,096
bytes while retaining read migration for v1/v2 current slots, checkpoints and
the protected baseline. EXF1 uses the same larger ceiling. Cliptick now has a
bounded on-device compiler profile for C, Rust and native Go; host `expos` can
also emit the same runtime-free Go image. This is a real x86-64 EXF1 build and
Ring 3 execution path, but not GCC, rustc, or the upstream Go runtime/toolchain.

The driver side of network-stack isolation is also implemented: the sealed
built-in worker owns an RTL8139 polling driver and one private writable/NX DMA
aperture; ordinary Forms, Python and the HTTP parser cannot obtain its Network
Handle. Ring 0 validates and pins the aperture, substitutes its physical
addresses into RBSTART/TSAD, admits only exact RTL8139 register/width/value
operations and revokes PCI bus mastering before the arena is released. The
production DNS path now runs ARP neighbor
resolution plus IPv4, UDP and bounded DNS query/response parsing in that Ring 3
worker; DHCP Discover/Offer/Request/Ack construction and parsing now run in the
same isolated worker. Production kernel DHCP and DNS packet parsing has been
removed. Plain HTTP's bounded one-shot TCP handshake, ordered receive,
retransmission and close use the same portal, with fresh per-invocation
connection identities. Authenticated HTTPS now keeps streaming TCP, TLS 1.3
records, certificate-chain/validity/signature checks and decrypted response
bytes in that worker. ICMP echo and generic UDP also construct, checksum and
validate packets there. The production kernel retains protocol admission,
reply revalidation, PCI discovery and the confined register/DMA lease portal.
The release ABI no longer exposes its former raw-frame copy path. Full
protection from a malicious or faulty DMA-capable physical device still
requires an IOMMU.

Still open: physical-page sealing for dynamic Form code (its supervisor backing
alias remains writable/NX), dedicated IST fault stacks, stack guard pages,
hardware IOMMU enforcement, coverage-guided fuzzing, C interpreter sanitizers, and
TLS/ExpFS/Browser parser campaigns. Early bootstrap mappings precede section
hardening. CPUs without NX boot diagnostically with `wx=false`.

Acceptance: malicious Forms and service inputs cannot read/write supervisor
state, change another CFC, inherit ambient Handles or exceed their budget;
faulting one service leaves the desktop, storage and other Forms usable. Keep
BIOS, native UEFI, SMP and Genesis regression coverage.

## 3. Interrupt-driven hardware — very high

Baseline has MSI/MSI-X discovery/reservation/programming, but device completion
handlers are not armed; NVMe, AHCI, xHCI and RTL8139 poll.

Implement handlers with explicit vector ownership, device acknowledgement,
EOI, bounded completion queues, and race-safe sleep/wakeup. Drivers must not
take ordinary service locks from interrupt context. Preserve a bounded fallback
when an interrupt route cannot be established, reporting its use explicitly.

Bring up VirtIO-block, then VirtIO-net under QEMU; add a documented Intel
Ethernet device family, USB mass-storage BOT/SCSI transport, then Intel HDA
controller/codec discovery and playback. Each driver needs error, timeout,
reset, malformed completion and concurrent traffic tests. Verify that I/O
completes from interrupts and does not require the previous polling loop.
Keep physical-hardware claims separate from emulated device evidence.

## 4. Genesis installer — very high

Baseline includes GPT/FAT32 whole-disk installation, encrypted CFC state,
redundant metadata, readback verification and recovery. It selects storage by
probe order; partition-aware install and safe physical selection are absent.

First expose model, serial, capacity, transport and stable identity from each
storage backend. Require a concrete selected device and revalidate identity
before writes. Build a read-only partition inventory and an explicit extent
plan; enforce all writes against that plan. Add existing-partition and free-
space installation without touching unrelated partitions. Never imply that
CFC encryption encrypts the ESP or every physical disk sector.

Add graphical wizard pages for device/partition choice, CFC identity, packages,
encryption and a final exact write plan. Retain accessible keyboard/serial
paths. Automate corrupted-primary/backup metadata, interrupted installs,
wrong-password/tamper failures, protected-baseline recovery and preservation
of sentinel data in unrelated partitions. Physical installation remains
experimental until those paths and actual target hardware are validated.

## 5. Public Ayo repository — high; free GitHub hosting

The user approved this direction only if it can use free GitHub hosting.
Use a public GitHub repository, a static Pages index on the free `github.io`
domain, release assets for packages and standard public Actions runners. No
paid domain, paid runners or hosted server is required. Respect provider limits;
revisit the design if traffic grows beyond them. No repository or hosting
settings have been changed by this increment.

Baseline host Ayo already verifies pinned Ed25519 catalogs/artifact SHA-256,
resolves dependencies and has a journaled transaction/rollback path. The native
Ayo catalog and host JSON-state bridge are different paths. SDK packaging is
deterministic and now distinguishes host output from loadable EXF1 C/Rust Forms.
Native images can be imported manually; completing registry-driven installation
still requires connecting these paths.

Implement a registry builder/signer and documented contributor PR flow. Build
native executable Form artifacts twice from pinned inputs/toolchains and
compare digests. Validate dependency graphs, platform/ABI constraints and
declared Handles before signing. Keep maintainer keys outside Git, never expose
signing secrets to contributor builds, and publish only reviewed artifacts.
Pin registry trust in clients; downloaded metadata must never choose its own
trusted key. Add replay/version policy and key rotation before auto-update.

Acceptance: an independent contributor's executable Form installs from the
signed repository and runs in CPL3; tampering/wrong keys/missing dependencies
fail closed, and a failed or interrupted update preserves the previous usable
version. Storing `.form` metadata alone does not satisfy native execution.

Hosting references (checked 2026-10-09):

- [GitHub Pages limits](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits)
- [GitHub Actions billing](https://docs.github.com/en/actions/concepts/billing-and-usage)

## 6. ExpDisplay — medium

Add typed, bounded clipboard ownership and transfer between Forms; authorize
cross-CFC copies as explicit transfers, never shared identity/storage. Define
drag-and-drop offers with accept/cancel lifetimes, then route real PS/2/xHCI
wheel events. Add persistent workspaces and correct focus/window membership.
Improve glyph rendering with explicit Unicode/font bounds and text tests.

Separate logical display/surface coordinates from physical scanout before
multi-monitor support. Validate per-monitor geometry, cursor motion, clipping,
hot-unplug and window recovery. GPU acceleration remains deferred until driver
memory/DMA ownership has a safe architecture.

## Repeatable checks for the first increment

`make isolation-check` builds `build/isolation/probes.iso`, a test-only image.
It tests kernel WP/NX and SMAP with actual page faults, user-set AC entry,
Form exception containment, a supervisor fault during an active Form, and an
NX/SMEP/SMAP-disabled CPU. Logs are under `build/isolation/`. Fault recovery is
only linked with `isolation-self-test`; do not distribute this image.

`make parser-fuzz` runs four ignored host campaigns with 100,000 mutations
each, plus seed truncations. Network targets cover DHCP, IPv4, UDP, TCP, DNS,
HTTP/chunked framing and URLs; the other targets cover PCM WAV and Genesis
manifest/plan decoding and EXF1 image validation. Genesis also recomputes checksums to reach semantic
validation. Inputs are mutated from valid seeds with bit flips, boundary values,
insertions, truncations and random bytes. This is deterministic mutation testing,
not coverage-guided or sanitizer evidence. It never touches hardware.

Set `LOOP_FUZZ_CASES` and `LOOP_FUZZ_SEED` to change/repeat a campaign. A mutation
panic saves its bytes under `build/fuzz-findings/` and prints its seed and case.
Use `cargo test -p expos-kernel parser_fuzz_network -- --ignored --nocapture`
to rerun one target. Future fuzzing should expand corpus depth and coverage.

## First increment validation snapshot — 2026-10-10

The first increment passed these checks against QEMU or host fixtures:

| Check | Result |
|---|---|
| Workspace Rust tests | 241 passed; three campaign tests run separately |
| Target-specific kernel Clippy, including `isolation-self-test` | No warnings with `-D warnings` |
| Formatting and `git diff --check` | Passed |
| BIOS smoke, display, AC'97 PCM capture, session authority | Passed |
| Local HTTP/Browser/network fixture | Passed with localhost socket access |
| Persistence across reboot | Accounts, settings, content, Handles and checkpoints passed |
| NVMe/AHCI and USB-only HID | Read/write persistence and paced real USB input passed |
| Ayo and Rust/Go/C/Python SDK checks | Passed, including deterministic host packaging |
| Embedded Python and native Python execution | Passed |
| Form resource budgets and Ring 3/SMP/preemption | Passed |
| Native UEFI and single-user policy | Passed |
| Visible BIOS/UEFI startup and PS/2 input | Passed; UEFI desktop/console captures inspected |
| New isolation suite | Protected CPU, legacy CPU and fatal supervisor-fault scenarios passed |
| Parser campaigns | 100,000 mutations each for network, WAV and Genesis, plus truncations |
| Genesis Architect and encrypted Basic | Installed, disk-booted, unlocked/logged in and shut down; redundant metadata and protected baseline checked |
| Visible Genesis | Framebuffer prompt and real PS/2 input passed |

Regression fixtures also needed updates for the Mole baseline: explicit
launcher shortcuts, current Settings categories, acceptance of the existing
UEFI-to-Bochs mode reprogramming path, and a resource limit derived from actual
resident Form bytes. USB input is now paced and waits for command completion;
this does **not** demonstrate rapid input delivery during a busy kernel or an
interrupt-driven xHCI driver. Existing semantic assertions remain in place.

At that snapshot, the normal `build/kernel.elf` was checked to exclude fault-injection symbols
and recovery hooks. Build artifacts are development images, with no new
physical-machine installation claim:

| First increment artifact (superseded by subsequent builds) | SHA-256 |
|---|---|
| `build/expos.iso` | `ba4aac01ee6dc2e6217638d61576ca3d40d9c954960c14ee2d21708aa1d0439d` |
| `build/ExpOS-v9-x86_64.iso` | `e7f9e12a666d415801f8fcc30868548bd548f565083fb5e925559574d8477682` |

The initial full-suite invocation stopped on stale fixtures; after correcting
them, all constituent `make all` checks passed in staged runs, followed by
`genesis-check` and `genesis-display-check`. The new isolation and parser checks
are now dependencies of `make all`. Live Internet/TLS endpoints, additional
Genesis Custom/Paranoid variants and physical hardware were not rerun here.

## Second increment validation — 2026-10-10

EXF1 and the freestanding C SDK are implemented locally on the same development
branch. No commit, release or registry publication was made. The first snapshot
above records earlier work; these focused checks were rerun for this increment:

| Check | Result |
|---|---|
| Workspace Rust tests | 243 passed; four ignored campaigns run separately |
| SDK CLI | Four tests passed, including native output equality across two directories, import reconstruction, EXF1 package metadata, oversize/undefined-symbol rejection and host-run refusal |
| Parser campaigns | 400,000 mutations total across EXF1, network, WAV and Genesis, plus seed truncations |
| Target kernel Clippy including test feature, formatting, diff whitespace | Passed |
| Normal BIOS native-image suite | Compiled C logged and exited with result 42; data/BSS and persisted re-execution passed; malformed imports rejected; CPU instruction/mapping probes contained; GS-modifying loop stopped after 256 ticks; survivor succeeded |
| NX-disabled CPU | EXF1 refused; diagnostic capsule still ran |
| Isolation regression | Kernel WP/NX/SMAP, AC entry, Form faults, fatal supervisor fault and legacy CPU scenarios passed |
| Existing Ring 3 regression | CR3 isolation, SMP and timer preemption passed |
| Native UEFI | Firmware handoff, desktop initialization, four CPUs and Ring 3 capsule execution passed |
| Genesis | Architect and encrypted Basic install, redundant metadata, protected baseline, PS/2 disk boot, login and shutdown passed |

`build/kernel.elf` has no isolation-injection symbols. The compiled sample is
201 bytes (24-byte header, 145-byte code, 32-byte initialized data; eight extra
zero bytes). Its import script is `build/native-image/NativeHello.shell`; its
local Ayo archive is `sdk/examples/native-hello/dist/NativeHello-0.1.0.tar`.

| Current artifact | SHA-256 |
|---|---|
| `build/expos.iso` | `25c85930a339fc676ddff85e2050d94ee5d79bc1c4d806b40f199094dc46f5aa` |
| `build/ExpOS-v9-x86_64.iso` | `eb6cc3c395a00a295b2ee4099199ca28a235e1517a5df9ae4881ee4596f42b39` |
| `sdk/examples/native-hello/.expos/build/NativeHello` | `c91b5c6c705168560b58aa366d5d7052079c12786d4a8f0f5e25a82088e35d60` |

The complete `make all` suite and physical hardware were not rerun for this
increment. EXF1 execution is currently proven by the BIOS native-image suite;
the UEFI regression executes the existing capsule path. Interrupt-driven device
completion, larger isolated interpreter/network services, the public registry,
partition-aware Genesis and the requested ExpDisplay work remain pending.

## Third increment — 2026-10-11 — isolated services and Cliptick

MicroPython now links into a separate, built-in ELF service rather than the
supervisor image. The real VM enters Ring 3 through the native Form runtime.
Each invocation owns a 2 MiB arena, RX code, RO/NX input, RW/NX data and a
128 KiB stack. Only Execute and bounded reply Handles are issued; each context
is sealed. Replies cross the ABI in chunks of at most 512 bytes, with a 32 KiB
capture ceiling. Worker faults and hardware time budgets return errors and
release the arena. Existing Python source, VM heap and opcode limits remain.

HTTP response parsing uses another built-in Ring 3 worker. The kernel validates
its serialized reply before consuming lengths, status or header strings. A
later fourth-increment worker now owns the RTL8139 polling driver, private DMA
buffers, DHCP, DNS, ICMP, generic UDP, plain TCP and HTTPS streaming TCP/TLS.
Ring 0 retains only PCI discovery, DMA-page validation/address substitution, a
register allowlist and bus-master revocation. Software driver and protocol
isolation is complete for the implemented IPv4 client paths; hardware IOMMU
confinement is not.
NX-disabled systems refuse the workers. The
external EXF1 wire format is unchanged; its admitted image ceiling is now
4,096 bytes.

Browser DOM construction is kept out of nested click/navigation stack frames;
large documents and HTTP responses are passed by owned pointers. Pages exceeding
the interactive node budget can display a bounded passive reader preview with
scripts, event handlers and external resources disabled. The browser remains
a bounded native HTML/CSS/script subset, with existing HTTPS trust restrictions.
This does not establish support for arbitrary modern websites.

Ayo now has 32 optional applications, including **Cliptick** and **Writer**.
Both share a bounded cursor-based editor: file sidebar, insertion and deletion,
arrow navigation, line numbers, dark/light themes, save/open and a status bar.
Text Forms support `.txt`, `.sys`, `.py`, `.rs`, `.go`, `.c` and `.h` without
renaming their extensions. `.sys` is text; opening it never executes it.
The ExpFS content ceiling is now **4,096 bytes per file**. Saving is transactional;
unsaved content blocks opening or creating another file. Ctrl+S saves.

Cliptick uses a VSCodium-inspired explorer/editor/output arrangement and runs
Python natively through the isolated VM (Run or Ctrl+R), capturing actual output
and exceptions. Its Settings group offers Normal/Operator intent and Sandbox
buffer/Local saved-source selection. Operator selection requires an existing
Operator session and resets when the desktop session ends; it cannot elevate a
Power or Guest session. Both modes remain Ring 3 with the same narrow worker
Handles. Local saves and reloads the CFC source before running; sandbox does not
save. Neither mode exposes filesystem/network imports. **C, Rust and Go use the
on-device freestanding compiler profile and execute its generated EXF1 Form
natively in Cliptick.** LOG output and the actual exit result are captured with
the same hardware fault/watchdog boundary. Local mode saves the executable
under the source base name; sandbox assigns a CFC-local ephemeral FIN. The host
SDK builds freestanding C, integer-only `no_std` Rust, and the matching bounded
native Go profile. The profile supports a main entry, one bounded log string
and an unsigned result; general language frontends and the upstream Go
runtime/toolchain remain pending.

`dmesg` has been removed. `bootlog [filter]` and `trace [filter]` expose the latest
128 actual kernel events, with 224 bytes per event and a busy-drop counter.
`trace browser`, `trace network`, `trace services`, `trace faults`, or an exact
marker substring narrow the output. Ordinary console input is not recorded.
`serviceinfo` reports worker requests, completions, faults, hardware budget stops,
rejected admissions and captured reply bytes. These commands also work in the
graphical Terminal. Service counters are for the current boot; logs rotate.

The shell welcome and Terminal startup are quieter. `style prompt full|compact|path`,
`style color default|green|amber|cyan`, `style cursor block|beam|underline`, and
`style spacing tight|roomy|wide` save CFC preferences. Prompt and color apply to
both shell and Terminal; cursor/spacing currently affect graphical Terminal.
`style reset` restores defaults. A failed persistence write is reported and the
style remains session-only. Existing graphical font/background controls remain.

### Third increment validation

| Check | Result |
|---|---|
| Workspace Rust tests | 249 passed; five ignored mutation campaigns run separately |
| SDK CLI | Five tests passed, including reproducible freestanding Rust output in two directories and bounded import scripts |
| Host Ayo | All Go package tests passed; catalog contains Cliptick and Writer |
| Parser campaigns | 500,000 mutations across EXF1, service ELF, network, WAV and Genesis, plus seed truncations |
| Kernel target Clippy, formatting and diff whitespace | Passed |
| Network QEMU regression | HTTP, redirects, subresources, storage/cookies, node-limit reader fallback, DHCP and 160 pings passed |
| MicroPython QEMU regression | Real Ring 3 VM, exception/memory/opcode budgets, fresh-VM recovery and revoked Execute rejection passed |
| Worker fault probes | Illegal opcode, read-only input write and infinite loop were contained; a fresh Python VM succeeded after each |
| Isolation regression | Kernel WP/NX/SMAP, Form faults, legacy CPU and fatal supervisor-fault checks passed |
| Native C/Rust images | Both compiled samples executed; C data/BSS, persistence, hostile CPU instructions, W^X and non-yielding budget controls passed |
| Ring 3 regression | Per-Form CR3, SMP execution and timer preemption passed |
| Native UEFI | Firmware handoff, four CPUs, capsule execution and the isolated Python VM passed |
| Editor/browser PS/2 GUI | Ayo installation; cursor insertion; Python result 42; save/open `.py` in Writer; Normal/Operator and sandbox/local settings; repeated mouse clicks between small and oversized HTTP pages; clean shutdown passed |
| Reboot GUI | Catalog installation, saved Python source, local-run mode and theme survived reboot; Operator intent reset to Normal |
| Native Cliptick PS/2 GUI | Imported C and Rust ran in Ring 3 inside the IDE; results 42/43 and exact captured output counts 31/35 bytes passed |

### Fourth increment validation — 2026-10-12

| Check | Result |
|---|---|
| Workspace Rust tests | 96 core, 154 kernel and 2 SDK tests passed; five mutation campaigns remain explicitly ignored in the ordinary suite |
| SDK CLI | Six tests passed, including reproducible native Go output and bounded import/package behavior |
| Target Clippy, formatting and diff whitespace | Kernel and Ring 3 network worker passed with `-D warnings`; `cargo fmt --check` and `git diff --check` passed |
| 4 KiB ExpFS | Focused 565-byte and full 570-byte persistence flows wrote, inspected, rebooted and read back in QEMU; the unit round trip covers a 3,072-byte Form and v1/v2 migration remains accepted |
| Native images | C, Rust and Go builds, Ring 3 execution, data/BSS, faults, GS/preemption, reboot persistence and NX-disabled refusal passed |
| Graphical on-device compilers | Native Go compiled and ran from Cliptick; the complete editor/reboot check passed; focused C/Rust sources returned 42/43 with exact 31/35-byte output |
| Isolation | Protected, legacy-CPU and fatal supervisor-fault QEMU scenarios passed |
| Local network/Browser | DHCP, isolated plain-HTTP TCP, redirects, subresources, storage/cookies, reader fallback and 160/160 isolated ICMP pings passed; repeated worker restarts use fresh TCP connection identities |
| Ring 3 network worker | Local QEMU negotiated `10.0.2.15` through isolated DHCP and completed repeated TCP and 160/160 ICMP exchanges. RTL8139 state and private DMA buffers were recreated in Ring 3 for each worker; QEMU logs prove the allowlisted lease and bus-master revocation. Live `internet-check` resolved `example.com` through isolated DNS; live Wikipedia verified TLS 1.3 and delivered HTTP entirely through the worker. Generic UDP shares the same bounded packet path. Hardware IOMMU confinement remains absent |

Increasing Form contents exposed a real bootstrap-stack collision with the
high identity-map page-directory entries during desktop entry. The kernel
stack reservation is now 2 MiB, leaving a full extra MiB above those tables;
the exact previously faulting persisted-native/desktop sequence passes.

External Browser navigation is now queued by click/key handlers and executed
on the next shallow desktop-loop iteration. A combined linked-page stress case
passed external CSS/JavaScript, storage, cookies, script fetch, 24 content
cards, repeated full loads and node-limit reader fallback without retaining the
input hit-testing/compositor frames or faulting the kernel.

The compiled `browser_click` frame now reserves 1,144 bytes instead of the
previous roughly 147 KiB; `navigate_external_address` reserves 14,104 bytes
instead of roughly 307 KiB. Large DOM construction remains in separate bounded
helper frames. This removes the identified nested-stack pressure. The specific
original user URL was not supplied, so the passing local link fixtures do not
prove every Internet page or the exact reported crash is fixed.

Current count: **46,000 physical Rust lines in 47 files under `kernel/src`**,
including comments, blank lines and tests; **43,629 nonblank lines**. This count
excludes shared `crates/expos-core`, boot assembly, host SDK/tools and vendored
MicroPython. HTTP service code under `ports/services` is also outside that count.

| Artifact | SHA-256 |
|---|---|
| `build/expos.iso` | `c45a45cd0ffb28e82754ee83cf0388223b3f6312d1b4ea66af03720ed8038a09` |
| `build/kernel.elf` | `41a5ed172a44de5fc735c5d21bc05ca390e7aa4142cf2bf2541a083fb8702693` |
| `sdk/examples/native-rust/.expos/build/NativeRust` | `73f5d1c41497585c5fc828c9f7f7062ce4487a68ead36a4d25177f56af819248` |

GUI evidence is in `build/editor-check/`, `build/editor-reboot-check/`, and
`build/cliptick-native-check/`; serial regression logs are under `build/` and
`build/isolation/`. The final ISO's GUI and native IDE tests were rerun after
finishing the help/status text. Full `make all`, Genesis installer/recovery,
live public HTTPS endpoints and physical hardware were not rerun for this
increment. The Genesis ISO from the previous increment was not rebuilt.
Everything remains local and uncommitted on `codex/v9.3-loop`; HEAD and
`origin/main` remain the v9.2 Mole baseline `7121551`. No release or public
registry was published.
