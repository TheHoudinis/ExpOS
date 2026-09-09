# ExpOS v8

ExpOS is the Form-native HexaOS rebuild described by
[`docs/PHILOSOPHY.txt`](docs/PHILOSOPHY.txt). The current image contains:

- an x86_64 Multiboot2 kernel with VGA, serial, PS/2 keyboard and mouse input;
- FIN identity, Dimensions, PIMP/DIESE policy, capability-scoped Form Handles,
  typed relationships and transactional HexaFS metadata;
- HexaDisplay, a 1920x1080 software compositor with Form-owned surfaces;
- a flat dark desktop with an application menu and taskbar, no default or
  pinned applications, and movable, closable, minimizable and maximizable
  windows;
- an interactive Settings control center for appearance, display, input,
  network, Wi-Fi, Bluetooth, privacy and system behavior;
- readable case-sensitive 8x8 framebuffer text and an expanded 8x16 VGA
  console font;
- Ayo v3 package installation, verification, ownership, rollback and recovery;
- a capability-gated RTL8139 network path with Ethernet, ARP, static IPv4,
  ICMP, UDP, DNS A lookup, one bounded TCP client and HTTP/1.0 GET;
- a graphical Browser that fetches and renders bounded `http://` documents;
- graphical and console login for Operator, Power and Guest authority.

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
make ayo            # test and build Ayo v3
make all            # run the complete native test suite
```

Run the image with:

```sh
make run
```

Choose `1` for the graphical environment or `2` for the console. The built-in
development accounts are:

| User | Password | Authority |
|---|---|---|
| `operator` | `expos` | Operator |
| `developer` | `prism` | Power |
| `guest` | `guest` | Guest |

These accounts are for development only. Runtime account changes are not
persistent.

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

The graphical Terminal supports bounded command history with Up and Down. In
Browser, click the address field or press `/`, type a plain `http://` URL, and
press Enter.

Settings controls are clickable and keyboard-accessible. Appearance, taskbar,
status-area, border, contrast and pointer-speed changes take effect immediately
and remain active for the current boot. The Network switch is enforced by the
native packet path through a requester-bound Configure Handle; it is not a
painted UI flag.

## Network and Browser

QEMU provides the current static guest configuration: `10.0.2.15/24`, gateway
`10.0.2.2`, and DNS server `10.0.2.3`. Network I/O is authorized at the Driver
Form boundary with a requester-bound Network Handle.

The native stack supports:

- RTL8139 polling DMA, Ethernet and ARP;
- static IPv4 and ICMP echo;
- checksum-validated UDP and DNS A records;
- one synchronous, bounded outbound TCP connection;
- bounded HTTP/1.0 GET requests and Browser rendering for `http://` URLs.

Use `ifconfig`, `ping`, `dns`, `fetch` and `netstat` in the console to inspect
and exercise the network. Type an `http://` URL in Browser to fetch it through
the same native stack.

Settings reports Ethernet carrier state and separately reports Wi-Fi and
Bluetooth hardware presence, driver state and requested power state. ExpOS
does not claim a connection when an adapter or driver is unavailable.

This is not a modern standards-complete browser. HTTPS/TLS, DHCP, IPv6,
physical Wi-Fi drivers, a USB host/Bluetooth data path, concurrent sockets, TCP
servers, CSS, JavaScript, cookies, downloads and media decoding are not
implemented. HTTP response and document sizes are fixed and bounded.

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

Form state, accounts and PIMP changes remain in memory until native HexaFS block
persistence is connected.

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
