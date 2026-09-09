# Architecture status

The rebuild is split into a small trusted semantic core and platform adapters.
Host tests exercise the same `no_std` core that is linked into the kernel.

```text
firmware / GRUB (temporary)
          |
          v
  x86_64 bootstrap kernel
          |
          v
 FIN -> Form Registry -> Dimension Binding
          |                    |
          v                    v
     Form Handle <--- capability decision
          |
          v
 PIMP specification -> DIESE resolution
          |
          v
 HexaFS transaction + journal sequence
```

## Implemented vertical slice

1. The loader validates long-mode support and enters an identity-mapped x86_64
   kernel with a 512 KiB bootstrap stack.
2. The kernel validates Multiboot2, initializes polling serial and VGA output,
   and invokes `hexa_core::bootstrap_demo`.
3. The demo creates a Root Form and Stable Dimension with independent FINs.
4. A binding makes the Root Form visible in Stable at revision 1.
5. Typed PIMP settings request service mode, restricted networking and
   isolation. DIESE resolves them using explicit scope precedence and rejects
   same-scope conflicts with a diagnostic.
6. The capability broker returns a time-limited, Dimension-scoped Form Handle
   that authorizes only read and execution.
7. HexaFS preflights and atomically publishes a journaled metadata transaction.
8. The kernel starts a graphical Session Manager and maps the selected identity
   to Operator, Power or Guest authority before opening the command environment.
   Polling COM1, PS/2 keyboard and PS/2 mouse input work before the interrupt
   subsystem is ported. Commands can
   create and inspect Forms, change lifecycle state, grant or revoke Handles,
   validate PIMP specifications, inspect system state, reboot, and shut down.

## Trust boundaries

- Exact resolution uses FIN. Human-readable names are secondary and scoped by
  a Dimension binding.
- Operator, Power and Guest are authority inputs to capability decisions; they
  are not Unix UID aliases.
- Handles carry requester and target FINs, allowed operations, Dimension,
  expiry and revocation state. No file descriptor abstraction appears in the
  core. A Handle can derive only narrower children: delegation cannot add an
  operation, extend expiry, or change target/Dimension, and revoking a parent
  recursively revokes its descendants.
- Relationships are typed, FIN-to-FIN and optionally Dimension-scoped; package
  dependencies use the same model instead of paths.
- Ayo v3 maps verified registry metadata into Package Forms and materializes
  raw, tar or tar.gz artifacts beneath an explicit user-owned root. Downloads,
  extraction, owned-file receipts, state publication, uninstall and recovery
  share an atomic transaction; scripts, symlinks and path traversal are denied.
- HexaDisplay uses a Wayland-like ownership model without copying Wayland's
  Unix socket/file-descriptor ABI: clients own surfaces and Buffer Handles,
  mutate pending state, report damage, and publish atomically with `commit`.
  Focus, configure, frame-complete, key and pointer events are routed back to the owning
  FIN. The software renderer targets a 1920x1080x32 XRGB scanout in QEMU
  standard VGA's 16 MiB linear framebuffer BAR. The bootstrap maps the entire
  fourth-GiB PCI window, and HexaDisplay checks the adapter's active geometry,
  stride and required byte count before the first framebuffer write.
- The desktop creates separate Browser, Terminal, Forms, Packages,
  Settings, System, Games and Notes surfaces, plus Root, taskbar and launcher
  surfaces. Its flat dark shell provides a compact application menu and bottom
  taskbar without copied third-party assets, promotional copy or instruction
  footers. It starts with no open or pinned apps and supports focus, dragging,
  minimize, maximize, close and reopen. Each application receives a child Handle containing
  only Display and Input rights; the compositor checks it before visibility,
  geometry, commit or key routing. Leaving graphics restores the VGA mode 3
  register set before the kernel redraws its text console.
- The PS/2 adapter enables the auxiliary device, validates ACKs and decodes
  synchronized three-byte packets. The compositor clamps a save-under cursor,
  hit-tests the topmost visible surface and checks its Input Handle before
  routing motion or button events. The buffer protocol represents XRGB8888,
  ARGB8888 and RGB565 clients. Solid fills use clipped row writes; gradients,
  alpha blending, rounded rectangles and Bresenham lines are native primitives;
  cursor movement repaints only the cursor bounds; games repaint only the active window.
- Session identity is intentionally separate from a Unix UID. The current
  built-in accounts and twelve-slot runtime registry select a DIESE authority
  input and gate shell mutations. Operators can create/delete accounts and
  change passwords; password-bearing commands are omitted from shell history.
  Persistent account Forms, salted password hashes and lockout policy remain
  future storage/security work.
- Network is a Driver Form protected by requester-bound Network Handles and
  PIMP policy. Its current polling RTL8139 path implements Ethernet, ARP,
  static QEMU-user IPv4, ICMP echo, checksum-validated UDP, DNS A lookup, one
  bounded synchronous TCP client and HTTP/1.0 GET. DHCP, IPv6, TLS, physical
  Wi-Fi, concurrent sockets, TCP servers and interrupt-driven I/O are explicitly
  future work.
- Connectivity settings target a distinct Radio FIN through a requester-bound
  Configure Handle. The manager keeps software policy, PCI/USB presence, driver
  readiness and connection state separate. Disabling Network is checked by the
  real packet authorization path. PCI Wi-Fi/Bluetooth functions are discovered,
  while missing 802.11 drivers and the absent USB host stack remain visibly
  unavailable instead of being reported as connected.
- The Browser is an Interface Form above HexaDisplay. Its current document
  engine accepts local `hexa://`, `data:text/html` and bounded `http://`
  resources. It receives a requester-bound Network Handle only for non-Guest
  sessions when PIMP networking is enabled. HTTPS/TLS, CSS and JavaScript are
  not claimed.
- Go ABI v1 gives Go clients stable call numbers and request/response layouts
  for Forms, Handles, display surfaces, events, browser navigation and package
  transactions. The Go SDK and emulator are runnable today; native Go binary
  loading still depends on the execution-context loader and scheduler.
- PIMP accepts only known keys and typed values. DIESE never silently resolves
  an equal-precedence conflict.
- HexaFS transaction commit validates all staged records and capacity before
  publishing any record under a single journal sequence.

## Transitional boundaries

The current GRUB/Multiboot2 path came from expodOS and is explicitly temporary;
the philosophy calls for UEFI in Phase 1. The `ayo` JSON store is similarly a
host-development bridge that makes transactions inspectable. It serializes
updates with a lock, pending journal, atomic rename and recovery snapshot, but
is not the native persistent format and will be replaced by kernel Form Handle
calls.

The 32-bit alpha is quarantined under `legacy/alpha32`. Its code may be ported,
but its paths, owner/group modes, file descriptors, sudo-like ACL behavior and
process naming must not leak into the new public model.

The command environment is deliberately backed by fixed-capacity, in-memory
tables at this stage. Its mutations exercise the core semantics but are not
durable until the HexaFS block driver and recovery path are connected.

Alpha.12 includes the 1920x1080 empty-start desktop, normal case-sensitive text,
Notes, dual graphical and console login selection, Ayo v3 artifact transactions,
and capability-gated native RTL8139/ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP networking.
The Diamond II build is
also exposed through `make run-alpha`, providing a runnable migration fallback
for networking, scheduling, ATA persistence and the games not yet redesigned
around v8 semantics.
