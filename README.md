# ExpOS v8

ExpOS is the Form-native operating system described by
[`docs/PHILOSOPHY.txt`](docs/PHILOSOPHY.txt). The current image contains:

- an x86_64 kernel with native UEFI handoff and a separate Multiboot2 fallback,
  VGA, serial, PS/2 keyboard and mouse input;
- Operator-authenticated single-user maintenance and normal multi-user startup;
- ExpPython, a bounded embedded MicroPython interpreter, plus a host Python SDK;
- FIN identity, Dimensions, PIMP/DIESE policy, capability-scoped Form Handles,
  typed relationships and a disk-backed transactional ExpFS Form graph;
- a bounded semantic CFC core with a distinct `CfcFin`, required name and
  non-replaceable Primary Dimension, exclusive cross-CFC identity ownership,
  CFC-owned ExpFS transactions/Handles, and CFC-bound metadata for an
  up-to-eight-checkpoint ring plus a separate installation-baseline descriptor;
- core ExpScope deny-by-default reachability, broker-lifetime ExpSeal
  root-handle cutoff with strictly attenuated delegation, and fixed-capacity
  ExpBudget accounting;
- Form-native execution contexts and cooperative scheduler admission keyed by
  CFC/Dimension/FIN, carrying an address-space descriptor, Handle set,
  ExpBudget, event queue, memory/CPU state, runtime and dispatch accounting;
  persisted `executable` Forms run bounded ExpPython payloads while their FIN
  context owns the cooperative scheduler slice and retain an exit result;
- bounded Form-native kernel controls: typed tunables, signal/timer/resource
  readiness watches, and shell-runtime-local resource accounting with DIESE-gated
  mutation;
- ExpDisplay, a runtime-selectable 640x480, 1280x720 or 1920x1080 software
  compositor with Form-owned surfaces, atomic commits, presentation-complete
  frame events, bounded damage-region scanout, double-buffered Bochs/QEMU
  output and selectable 60, 75, 120 or 144 Hz pacing; fresh state starts at
  the lowest safe choices, 640x480 and 60 Hz;
- a flat dark desktop with an application menu and taskbar, no default or
  pinned applications, and movable, closable, minimizable and maximizable
  windows;
- an interactive Settings control center for appearance, windows, taskbar,
  display, performance, input, network, Wi-Fi, Bluetooth, privacy and system
  behavior, including six themes, seven procedural wallpapers, four cursor
  themes, five font faces and three real font weights;
- readable case-sensitive 8x8 framebuffer text with runtime face/weight
  rasterization and an expanded 8x16 VGA console font;
- Ayo v3 package installation, verification, ownership, rollback and recovery;
- a capability-gated RTL8139 network path with Ethernet, ARP, DHCP IPv4,
  ICMP, UDP, DNS A lookup, one bounded TCP client and HTTP/1.0 GET over plain
  TCP or authenticated TLS 1.3;
- a graphical Browser that fetches and renders bounded `http://` and
  `https://` documents, applies a small native CSS subset, runs a deterministic
  DOM-mutation/click-handler JavaScript subset, and searches DuckDuckGo's
  non-JavaScript HTML endpoint from the address bar;
- graphical and console login for Operator, Power and Guest authority;
- dedicated ATA PIO persistence with alternating verified ExpFS CFC snapshots
  for arbitrary Forms/content/Dimensions/relationships/revisions/PIMP state,
  accounts, and desktop settings; the old EXPOST03 slots are read-only migration
  input; eight full-state rotating checkpoints can be listed and restored.

## Approved Genesis target

The architecture now defines every Central Inflation Fabric (CFC) as an exclusively
owned ExpOS environment with its own typed FIN, required nonempty name, and
exactly one Primary Dimension. Forms, data, Dimensions, relationships,
capabilities, identities/policy, storage extents, keys, and checkpoints cannot
be shared across CFCs; an explicit transfer creates a new destination-owned
entity.

Each encrypted CFC receives an independent random storage key. Argon2id derives
a key-encryption key (KEK) from the Operator password to wrap and unlock that
storage key, and persistent CFC state uses authenticated encryption (AEAD).
Recovery retains eight rotating checkpoints plus a protected immutable
installation baseline per CFC. Exact AEAD selection, Argon2id parameters,
key-envelope/nonce layout and authenticated/protected checkpoint encoding remain
implementation work; the current CRC format already performs header-last
rotation for current state and eight full-state checkpoint slots.

Native UEFI is the default Genesis and normal boot path; BIOS remains a
compatibility, recovery, and development fallback. **Architect** is the official
name of the configurable installation path (“expert” is only a legacy
explanation). The approved enforcement model names are **ExpScope** for
CFC/Dimension-aware confinement, **ExpSeal** for monotonic capability reduction,
and **ExpBudget** for per-context resource enforcement.

These invariants now have a first end-to-end Genesis vertical slice. `make
genesis-iso` builds `build/ExpOS-0.9-x86_64.iso`, a native-UEFI hybrid image.
Its Architect path requires exact `ERASE` confirmation, constructs primary and
backup GPT metadata, creates a FAT32 EFI System Partition, installs the runtime
at `EFI/BOOT/BOOTX64.EFI`, mints and persists CFC/Primary Dimension identities,
and stores only a salted password verifier for the initial Operator. `make
genesis-check` installs to a blank disk, reboots without the ISO, logs in with
that Operator, and shuts down.

This is deliberately not yet the Basic installer: Basic remains gated because
its mandatory Argon2id-wrapped storage key and AEAD ExpFS are not implemented.
The current Architect slice claims the whole ATA primary-master disk and is
explicitly unencrypted. It is QEMU/OVMF-tested; safe physical-disk selection,
AHCI/NVMe/VirtIO-block, USB input/media, BIOS installation, Secure Boot, and a
graphical installer remain open.

The bounded `Cfc`/`CfcCatalog` model enforces exclusive ownership of registered
Form and Dimension FINs. ExpScope snapshots that ownership, recovery artifacts
and ExpFS records are CFC-bound, and Handles carry a CFC identity. Native Form
mutations now commit and recover a complete bounded Form graph through ExpFS,
so a newly created Form and its content survive a reboot without a Form-specific
persistence path. Accounts and settings now load from and commit through the
same ExpFS current-state transaction; EXPOST03 is only a boot-time migration
fallback. Typed execution contexts now reach scheduler admission and
per-context budget accounting. Native checkpoint capture/restore and the
Architect Genesis construction path are live; the protected installation baseline, Argon2id key
wrapping, AEAD storage, real page-table switching, interrupts/preemption, and
user-mode CPU context switching remain implementation work.

The 32-bit Diamond II source remains isolated under `legacy/alpha32/` as a
buildable migration reference.

## Build

Requirements: Rust with the `x86_64-unknown-none` target, Cargo, NASM, GNU
binutils, GRUB i386-pc modules, xorriso, QEMU x86_64, Go, Make and Python 3.
GCC builds the vendored ExpPython runtime; Clang and OVMF are needed for native
UEFI builds/tests. Python also supplies the host SDK and test harnesses.

```sh
make uefi           # build build/esp/EFI/BOOT/BOOTX64.EFI
make genesis-iso    # build build/ExpOS-0.9-x86_64.iso
make genesis-check  # install, disk-boot, log in and shut down under OVMF
make run-uefi       # boot the native firmware path with OVMF
make bootmode-check # verify single-user account/service restrictions
make startup-check  # verify visible UEFI/BIOS screens and real keyboard login
make python-check   # verify Python scripts and limits inside the kernel
make iso            # build the fallback build/expos.iso
make check          # boot and exercise the command environment
make display-check  # verify desktop and window lifecycle
make network-check  # verify ICMP, TCP, HTTP and Browser against a local fixture
make internet-check # verify live DNS and public HTTP (requires Internet access)
make https-check    # verify TLS and fetch youtube.com HTML (requires Internet)
make search-check   # verify DuckDuckGo HTML address-bar search (requires Internet)
make persistence-check # verify Forms, Handles, settings and checkpoints across boots
make ayo            # test and build Ayo v3
make all            # run the complete native test suite
```

Run the image with:

```sh
make run
```

`make run` creates `runtime/expos-state.img` once and attaches it as the
primary ATA disk. ExpFS Form state, accounts, and customization changes are
journaled there. The
image is intentionally preserved by `make clean`; copy it to back up the
current local state.

The default launch uses native UEFI with OVMF. `make run-bios` retains the
previous GRUB image as the approved BIOS compatibility, recovery, and
development fallback. Native UEFI remains the primary architecture.

ExpOS discovers the framebuffer's PCI address assigned by firmware, so both
OVMF and SeaBIOS draw to the actual video memory. `make startup-check` captures
the chooser, login and desktop screens and signs in through emulated PS/2
keyboard events. Screenshots and logs are saved under `build/startup-uefi/`
and `build/startup-bios/`.

The boot chooser and graphical login explicitly present their completed back
buffer before waiting for input. If a saved display mode is rejected, ExpOS
automatically retries 480p before falling back to the console. For manual
recovery, choose Console, sign in as `operator`, run `displaydiag` and
`stateinfo` (or combined `diag`), then run `safevideo`. That saves 480p, 60 Hz
and VSync on; if the state disk cannot be written, the same safe settings stay
active for the current boot. `desktop` retries graphics without rebooting.

Choose `1` for the multi-user graphical environment, `2` for the multi-user
console, or `3`/`S` for Operator-only single-user maintenance. Single-user mode
disables networking and the desktop until restart. See [native boot details](docs/BOOT.md)
and [Python support](docs/PYTHON.md). The built-in
development accounts are:

| User | Password | Authority |
|---|---|---|
| `operator` | `expos` | Operator |
| `developer` | `prism` | Power |
| `guest` | `guest` | Guest |

These accounts and their short passwords are retained for development
compatibility. Newly set passwords must contain 8-23 printable characters.
Accounts and password changes persist when the state image is available.
Passwords are never stored as plaintext or reversible ciphertext: ExpOS stores
a per-account salt and a PBKDF2-HMAC-SHA256 verifier with 25,000 rounds.
Password arguments are masked while typed and omitted from shell history.
This describes the current account-login verifier only. It is distinct from the
approved future Argon2id KEK used to wrap a random per-CFC storage key; the
current state image is not storage-encrypted.

## Desktop

The desktop starts empty. Open the application menu from the taskbar or with
`Super+Space`. Only running applications appear on the taskbar. Windows contain
a title and standard minimize, maximize and close controls; promotional labels,
capability identifiers and instruction footers are not shown.

Useful keys:

```text
Super+Enter     Terminal
Super+B         Browser
Super+Tab       Next running application
Super+Up        Maximize
Super+Down      Restore or minimize
Super+Q         Close
Esc             Return to the console
```

The graphical Terminal has 40 lines of scrollback, 24 history entries with Up
and Down recall, and commands for identity, status, display, networking,
applications, users and basic shell-style operations. Run `help` inside it for
the exact list. Its `display` command reports the active and requested modes,
presentation policy, frame-pacing and scanout counters. The console
`displayinfo` command summarizes the selected presentation target and
VSync/page-flip state; `displaydiag` also reports submitted and copied damage,
damage-collapse, page-flip-failure and bounded vertical-retrace-timeout
counters. `timers` reports the TSC clock source used by the frame pacer.
`neofetch` works in both graphical and console terminals and draws the ExpOS
two-eye mark before the current system/session facts. Graphical `windowreset`
returns every application window to its default recoverable position. In
Browser, click the address field or press `/`, type an `http://` or `https://`
URL, and press Enter. Text without a scheme is treated as a search query and
sent to DuckDuckGo's non-JavaScript HTML search; prefix a query with `?` for the
same behavior.

Settings controls are clickable and keyboard-accessible. Its category and row
viewports follow the selection, so all eleven pages and long Windows/Appearance
lists remain usable at 480p. Appearance, taskbar, status-area, border,
contrast, pointer-speed, theme, wallpaper, cursor and font changes take effect
immediately and persist. Five allocation-free bitmap faces and Light, Regular
and Bold stroke weights preserve the existing text layout while visibly
changing glyph rendering.

The Windows page controls corner radius, border width, titlebar height, backdrop
and titlebar opacity, bounded off-screen travel, edge snapping, snap distance and
click/sloppy/pointer focus. Off-screen travel is configurable from contained to
256 px; movement remains bounded so a recovery strip or titlebar stays
reachable. The Taskbar page controls bottom/top/left/right placement, nine
panel sizes, start/center/end app alignment, edge-reveal auto-hide,
translucency, horizontal app labels and hardware-RTC seconds. The cursor shadow
is independently switchable. Across Appearance, Windows and Taskbar, 106
selectable values are directly wired to rendering, geometry or interaction.
The compact persistent-state extension validates 112 accepted states across
its selectors and booleans, including 28 reserved interaction states that are
stored but not advertised as working desktop controls. These counts overlap and
are not additive.

Resolution can be selected as 480p (640x480), 720p (1280x720) or 1080p
(1920x1080); it is saved immediately and applied when the desktop is reopened.
A fresh state image defaults to 480p and 60 Hz. Presentation pacing can be
selected as 60, 75, 120 or 144 Hz and VSync can be enabled or disabled; both
settings also persist. These rates are compositor frame targets, not physical
monitor modes or a claim that QEMU changed the host display's refresh rate.
With VSync enabled, ExpDisplay performs a bounded VGA vertical-retrace wait
before its Bochs framebuffer page flip; a timeout is recorded instead of
hanging the kernel. The Network switch is enforced by the native packet path
through a requester-bound Configure Handle; it is not a painted UI flag.

Open **Settings > Performance** to choose the renderer's cost/latency tradeoff.
Window shadows and procedural wallpaper effects are independent toggles.
**Efficient** presentation follows the selected software cadence for every
frame; **Responsive** permits only partial damaged commits to bypass that
cadence, while full repaints remain paced and the separate VSync preference is
still honored. Fresh state uses Efficient presentation with shadows and
wallpaper effects off, so the desktop begins with the least expensive renderer
path. These choices persist. With the keyboard, `6` selects Performance, `j`/`k`
select a row, Enter or Space activates it, and `+`/`-` move choices.

ExpDisplay adopts compositor concepts also used by Wayland—client-owned
surfaces, pending state published by an atomic commit, explicit surface damage
and frame completion after presentation—but it is a Form-native protocol, not
a Wayland wire protocol or `libwayland` compatibility layer. Consecutive pure
mouse-motion packets are combined into a bounded compositor update; keyboard
input and mouse-button transitions are retained in order.

## Network and Browser

The RTL8139 Driver Form negotiates its IPv4 address, netmask, gateway, DNS
server and lease through DHCP. A conservative QEMU-user fallback remains if no
DHCP server replies. Network I/O and manual `dhcp` renewal are authorized at
the Driver Form boundary with a requester-bound Network Handle.

The native stack supports:

- RTL8139 polling DMA, Ethernet and ARP;
- DHCP-configured IPv4 and ICMP echo;
- checksum-validated UDP and DNS A records;
- one synchronous, bounded outbound TCP connection;
- bounded HTTP/1.0 GET requests and Browser rendering for `http://` URLs;
- authenticated TLS 1.3 and `https://` GET with hardware RDRAND entropy, SNI
  and hostname checks, RTC certificate-validity checks, and certificate-chain
  and signature verification;
- a fixed-capacity HTML document model, CSS tag/class/id and inline rules, and
  deterministic title/text/style/click-handler JavaScript operations;
- address-bar search through the canonical
  `https://duckduckgo.com/html/?q=...` endpoint, with at most three redirects,
  rejection of HTTPS-to-HTTP downgrades, and projection of up to eight result
  titles and links into the bounded document model.

Use `ifconfig`, `dhcp`, `ping`, `dns`, `fetch` and `netstat` in the console to inspect
and exercise the network. Type an `http://` or `https://` URL in Browser to
fetch it through the same native stack. `make https-check` performs a verified
fetch of the HTML returned by `https://www.youtube.com/`.

Settings reports Ethernet carrier state and separately reports Wi-Fi and
Bluetooth hardware presence, driver state and requested power state. ExpOS
does not claim a connection when an adapter or driver is unavailable.

The browser engine is deliberately bounded: the parser accepts at most 16 KiB
per document, while native HTTP/HTTPS fetches retain at most the first 14 KiB of
the response body. A document contains at most 48 parsed nodes, 32 CSS rules,
16 scripts and 12 click handlers. Its CSS subset covers colors, background,
border, font size/weight, display, visibility, margin, padding and text
alignment with tag/class/id specificity and inline styles. JavaScript is not
arbitrary ECMAScript; only deterministic document title, node text, supported
style, visibility and click-handler mutations run. External
stylesheet/script/image loading, general Web APIs, `fetch`, cookies,
local/session storage, media containers/codecs, audio/video output and GPU
acceleration are absent. Consequently, fetching YouTube HTML does **not** make
YouTube playback work.

IPv6, physical Wi-Fi drivers, a USB host/Bluetooth data path, concurrent
sockets, TCP servers and downloads are also absent. HTTP, TLS record and
certificate-chain buffers are fixed and bounded. The embedded Web PKI store is
not a general CA bundle: GlobalSign Root R1 is the default anchor, while
DigiCert Global Root G2 is selected only for `duckduckgo.com` and its
subdomains. Sites chaining to another root are not accepted.

## Ayo v3

```sh
./ayo/bin/ayo --authority operator
./ayo/bin/ayo --authority operator install TextLab
./ayo/bin/ayo files TextLab
```

Ayo accepts built-in, local or HTTPS catalogs; verifies configured SHA-256 and
Ed25519 metadata; extracts raw, tar and tar.gz artifacts beneath an explicit
root; records owned files; and supports uninstall and recovery. It does not run
package scripts or install links.

## Console

The command environment includes Form, Dimension, relationship, capability,
account, display, package, hardware, network and bounded kernel-control
commands. Run `help` for the current list. Password-bearing commands are
excluded from history.

The new kernel-control interfaces borrow proven concepts from FreeBSD, but are
original Form-native implementations: they do not copy FreeBSD code or expose
its ABI. `sysctl` reads typed named nodes and lets an Operator change the three
validated writable event controls. `kqueue`/`kevent` registers fixed-capacity
signal, timer and resource watches, then returns sequenced readiness records.
`expbudget` accounts event watches, IPC bytes, scratch pages, Form operations and live Form content bytes
against validated soft/hard/ceiling tuples. Event-watch accounting is wired to
the queue. Form mutation attempts and live content bytes are enforced by the
shell; rejected growth leaves the original content intact. IPC bytes and
scratch pages are explicit runtime-local reservations. These are not yet
global process limits. DIESE authorizes mutations through the revocable
bootstrap Handle: Configure is reserved for Operator, Execute is available to
Power and Operator, and Guest receives read-only access.

```text
sysctl -a
sysctl kern.event.batch=8
kqueue add signal 7
kqueue signal 7 42
kqueue poll
kqueue add timer 9 1000 1000
kqueue add resource scratch-pages
expbudget list
expbudget set scratch-pages 2 64
expbudget charge scratch-pages 3
kqueue poll
expbudget charge scratch-pages 2
expbudget release scratch-pages 2
```

The watch and ready rings each hold at most 16 entries, dispatch batch size is
bounded to 1-16, duplicate watches are rejected, and optional coalescing
accumulates missed timer expirations. These controls are runtime diagnostics
and accounting primitives, not FreeBSD compatibility, a preemptive scheduler,
or persistent system configuration. `budget` and `rlimit` remain compatibility
aliases for `expbudget`.

The dedicated state disk now carries alternating, CFC-bound ExpFS database
snapshots containing Forms, content, Dimensions, relationships, revisions,
PIMP network state, accounts and Settings preferences. The current format is
CRC-protected rather than AEAD-protected. It captures and restores eight
rotating full-state checkpoints; the separate protected installation baseline
is not yet stored on disk.

## Preserved implementation

The Diamond II fallback remains separately buildable and runnable:

```sh
make legacy-alpha-check
make run-alpha
```

No fallback component is treated as a native v8 interface until it has a
Form/FIN/Dimension and capability-safe boundary.
The feature pass described above does not modify `legacy/alpha32/`.

## Repository map

| Area | Purpose |
|---|---|
| `boot/`, `kernel/` | x86_64 bootstrap and kernel |
| `crates/expos-core/` | platform-independent Form semantics |
| `ayo/` | Go package manager and development storage bridge |
| `sdk/go/` | Go ABI client and host emulator |
| `docs/PHILOSOPHY.txt` | source architecture specification |
| `docs/ARCHITECTURE.md` | implementation and trust boundaries |
| `docs/MIGRATION.md` | migration map |
| `docs/FEATURE_COVERAGE.md` | implemented and missing features |
| `legacy/alpha32/` | preserved Diamond II source |

ASL's high-level role is defined, but its exact interfaces, bootstrap ordering,
and cross-architecture implementation remain unspecified and unimplemented.
