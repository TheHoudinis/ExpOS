# ExpOS v8

ExpOS is the Form-native HexaOS rebuild described by
[`docs/PHILOSOPHY.txt`](docs/PHILOSOPHY.txt). The current image contains:

- an x86_64 Multiboot2 kernel with VGA, serial, PS/2 keyboard and mouse input;
- FIN identity, Dimensions, PIMP/DIESE policy, capability-scoped Form Handles,
  typed relationships and transactional HexaFS metadata;
- HexaDisplay, a runtime-selectable 640x480, 1280x720 or 1920x1080 software
  compositor with Form-owned surfaces, atomic commits, presentation-complete
  frame events, bounded damage-region scanout, double-buffered Bochs/QEMU
  output and selectable 60, 75, 120 or 144 Hz pacing; fresh state starts at
  the lowest safe choices, 640x480 and 60 Hz;
- a flat dark desktop with an application menu and taskbar, no default or
  pinned applications, and movable, closable, minimizable and maximizable
  windows;
- an interactive Settings control center for appearance, display, performance,
  input, network, Wi-Fi, Bluetooth, privacy and system behavior, including six
  themes, seven procedural wallpapers and four cursor themes;
- readable case-sensitive 8x8 framebuffer text and an expanded 8x16 VGA
  console font;
- Ayo v3 package installation, verification, ownership, rollback and recovery;
- a capability-gated RTL8139 network path with Ethernet, ARP, static IPv4,
  ICMP, UDP, DNS A lookup, one bounded TCP client and HTTP/1.0 GET over plain
  TCP or authenticated TLS 1.3;
- a graphical Browser that fetches and renders bounded `http://` and
  `https://` documents, applies a small native CSS subset, runs a deterministic
  DOM-mutation/click-handler JavaScript subset, and searches DuckDuckGo's
  non-JavaScript HTML endpoint from the address bar;
- graphical and console login for Operator, Power and Guest authority;
- dedicated ATA PIO persistent state for accounts and desktop preferences.

The 32-bit Diamond II source remains isolated under `legacy/alpha32/` as a
buildable migration reference.

## Build

Requirements: Rust with the `x86_64-unknown-none` target, Cargo, NASM, GNU
binutils, GRUB i386-pc modules, xorriso, QEMU x86_64, Go, Make and Python 3.
Python supplies the deterministic host HTTP fixture used by the network test.

```sh
make iso            # build build/hexaos.iso
make check          # boot and exercise the command environment
make display-check  # verify desktop and window lifecycle
make network-check  # verify ICMP, TCP, HTTP and Browser against a local fixture
make internet-check # verify live DNS and public HTTP (requires Internet access)
make https-check    # verify TLS and fetch youtube.com HTML (requires Internet)
make search-check   # verify DuckDuckGo HTML address-bar search (requires Internet)
make persistence-check # verify accounts/settings across two boots
make ayo            # test and build Ayo v3
make all            # run the complete native test suite
```

Run the image with:

```sh
make run
```

`make run` creates `runtime/expos-state.img` once and attaches it as the
primary ATA disk. Account and customization changes are journaled there. The
image is intentionally preserved by `make clean`; copy it to back up the
current local state.

The boot chooser and graphical login explicitly present their completed back
buffer before waiting for input. If a saved display mode is rejected, ExpOS
automatically retries 480p before falling back to the console. For manual
recovery, choose Console, sign in as `operator`, run `displaydiag` and
`stateinfo` (or combined `diag`), then run `safevideo`. That saves 480p, 60 Hz
and VSync on; if the state disk cannot be written, the same safe settings stay
active for the current boot. `desktop` retries graphics without rebooting.

Choose `1` for the graphical environment or `2` for the console. The built-in
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
counters. `timers` reports the TSC clock source used by the frame pacer. In Browser, click
the address field or press `/`, type an `http://` or `https://` URL, and press
Enter. Text without a scheme is treated as a search query and sent to
DuckDuckGo's non-JavaScript HTML search; prefix a query with `?` for the same
behavior.

Settings controls are clickable and keyboard-accessible. Appearance, taskbar,
status-area, border, contrast, pointer-speed, theme, wallpaper and cursor
changes take effect immediately and persist. Resolution can be selected as
480p (640x480), 720p (1280x720) or 1080p (1920x1080); it is saved immediately
and applied when the desktop is reopened. A fresh state image defaults to 480p
and 60 Hz. Presentation pacing can be selected as 60, 75, 120 or 144 Hz and
VSync can be enabled or disabled; both settings also persist. These rates are
compositor frame targets, not physical monitor modes or a claim that QEMU
changed the host display's refresh rate. With VSync enabled, HexaDisplay
performs a bounded VGA vertical-retrace wait before its Bochs framebuffer page
flip; a timeout is recorded instead of hanging the kernel. The Network switch
is enforced by the native packet path through a requester-bound Configure
Handle; it is not a painted UI flag.

Open **Settings > Performance** to choose the renderer's cost/latency tradeoff.
Window shadows and procedural wallpaper effects are independent toggles.
**Efficient** presentation follows the selected software cadence for every
frame; **Responsive** permits only partial damaged commits to bypass that
cadence, while full repaints remain paced and the separate VSync preference is
still honored. Fresh state uses Efficient presentation with shadows and
wallpaper effects off, so the desktop begins with the least expensive renderer
path. These choices persist. With the keyboard, `6` selects Performance, `j`/`k`
select a row, Enter or Space activates it, and `+`/`-` move choices.

HexaDisplay adopts compositor concepts also used by Wayland—client-owned
surfaces, pending state published by an atomic commit, explicit surface damage
and frame completion after presentation—but it is a Form-native protocol, not
a Wayland wire protocol or `libwayland` compatibility layer. Consecutive pure
mouse-motion packets are combined into a bounded compositor update; keyboard
input and mouse-button transitions are retained in order.

## Network and Browser

QEMU provides the current static guest configuration: `10.0.2.15/24`, gateway
`10.0.2.2`, and DNS server `10.0.2.3`. Network I/O is authorized at the Driver
Form boundary with a requester-bound Network Handle.

The native stack supports:

- RTL8139 polling DMA, Ethernet and ARP;
- static IPv4 and ICMP echo;
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

Use `ifconfig`, `ping`, `dns`, `fetch` and `netstat` in the console to inspect
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

DHCP, IPv6, physical Wi-Fi drivers, a USB host/Bluetooth data path, concurrent
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
account, display, package, hardware and network commands. Run `help` for the
current list. Password-bearing commands are excluded from history.

Accounts and Settings preferences are persisted through the dedicated state
disk. General Form contents and PIMP changes remain in memory until native
HexaFS block persistence is connected.

## Preserved implementation

The Diamond II fallback remains separately buildable and runnable:

```sh
make legacy-alpha-check
make run-alpha
```

No fallback component is treated as a native v8 interface until it has a
Form/FIN/Dimension and capability-safe boundary.

## Repository map

| Area | Purpose |
|---|---|
| `boot/`, `kernel/` | x86_64 bootstrap and kernel |
| `crates/hexa-core/` | platform-independent Form semantics |
| `ayo/` | Go package manager and development storage bridge |
| `sdk/go/` | Go ABI client and host emulator |
| `docs/PHILOSOPHY.txt` | source architecture specification |
| `docs/ARCHITECTURE.md` | implementation and trust boundaries |
| `docs/MIGRATION.md` | migration map |
| `docs/FEATURE_COVERAGE.md` | implemented and missing features |
| `legacy/alpha32/` | preserved Diamond II source |

ASL is not implemented because its established specification was not supplied.
