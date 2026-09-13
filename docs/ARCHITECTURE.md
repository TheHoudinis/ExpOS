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
   kernel with a 1 MiB bootstrap stack.
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
9. A bounded primary-master ATA PIO driver loads a dedicated ExpOS state image.
   Accounts and desktop preferences are recovered from the newest valid of two
   CRC-protected journal slots and mutations alternate slots by generation.

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
  FIN. The software renderer can program 640x480, 1280x720 or 1920x1080 XRGB
  scanout in QEMU standard VGA's 16 MiB linear framebuffer BAR. The bootstrap
  maps the entire fourth-GiB PCI window, and HexaDisplay checks the selected
  geometry, stride and double-buffer byte count against the aperture before the
  first framebuffer write. When the adapter accepts a virtual height of twice
  the visible height, rendering targets the hidden page and presentation flips
  the VBE Y offset. After a flip, only declared damage rectangles are copied to
  the newly hidden page, keeping the two pages coherent without a full-screen
  copy for cursor and terminal updates. Optional VSync performs a bounded
  legacy-VGA retrace wait; failed waits increment a diagnostic counter instead
  of blocking forever.
- The desktop creates separate Browser, Terminal, Forms, Packages,
  Settings, System, Games and Notes surfaces, plus Root, taskbar and launcher
  surfaces. Its flat dark shell provides a compact application menu and bottom
  taskbar without copied third-party assets, promotional copy or instruction
  footers. It starts with no open or pinned apps and supports focus, dragging,
  minimize, maximize, close and reopen. Settings offers six renderer-defined
  themes (including Aurora and Rose), seven procedural wallpapers (including
  Aurora and Mesh), four cursor themes and the three display presets. Its
  Display page also selects a 60, 75, 120 or 144 Hz compositor presentation
  target and optional VSync. These are software-pacing targets, not negotiated
  physical monitor modes. The graphical Terminal keeps bounded scrollback and
  command history and exposes identity, system, display, network and
  application commands.
  Each application receives a child Handle containing
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
- Presentation pacing uses a calibrated TSC frequency when CPUID supplies one
  and a conservative fallback otherwise. Fractional frame periods are carried
  without cumulative integer drift, clock discontinuities are recoverable, and
  the graphical Terminal's `display` command reports pacing misses. The console
  `displayinfo` command reports page-flip/frame/retrace-timeout state, the
  graphical System surface reports frame count, and `timers` identifies the
  clock source.
- Session identity is intentionally separate from a Unix UID. The built-in
  accounts and twelve-slot registry select a DIESE authority input and gate
  shell mutations. Operators can create/delete accounts and change passwords;
  password arguments are masked while typed and password-bearing commands are
  omitted from shell history. Account records persist to the dedicated state
  image. Passwords are represented only by a
  per-account salt, PBKDF2-HMAC-SHA256 verifier and round count (25,000 rounds
  for new records); candidate hashes are compared in constant time and
  plaintext input is wiped after use. This is not yet a claim of lockout,
  hardware-backed keys or a complete modern account-security policy.
- The state disk is a compact versioned store, not HexaFS. Two 2 KiB slots at
  fixed LBAs hold accounts and desktop preferences. Each commit writes the
  inactive slot with a monotonically advancing generation, format/version
  fields and CRC-32 over its header and payload; boot selects the newest valid
  slot, preserving the previous generation across an interrupted or corrupt
  write. Settings persist display mode, theme, wallpaper, cursor, accent,
  backdrop, pointer speed, presentation rate, VSync and desktop/connectivity
  flags. Older compatible records default to 60 Hz with VSync enabled. Form
  records, notes and PIMP revisions are outside this store and remain volatile.
- Network is a Driver Form protected by requester-bound Network Handles and
  PIMP policy. Its current polling RTL8139 path implements Ethernet, ARP,
  static QEMU-user IPv4, ICMP echo, checksum-validated UDP, DNS A lookup, one
  bounded synchronous TCP client and HTTP/1.0 GET over plain TCP or
  authenticated TLS 1.3. The TLS client requires hardware RDRAND entropy,
  sends SNI, verifies the requested hostname, certificate time validity from
  the RTC, the chain and signatures, and accepts a fixed AES-128-GCM-SHA256
  suite. TLS record and chain storage are fixed-size. Its Web PKI trust store
  is not a general operating-system CA bundle: GlobalSign Root R1 is the
  default anchor, and DigiCert Global Root G2 is selected only for
  `duckduckgo.com` and its subdomains. DHCP, IPv6, physical Wi-Fi, concurrent
  sockets, TCP servers and interrupt-driven I/O are explicitly future work.
- Connectivity settings target a distinct Radio FIN through a requester-bound
  Configure Handle. The manager keeps software policy, PCI/USB presence, driver
  readiness and connection state separate. Disabling Network is checked by the
  real packet authorization path. PCI Wi-Fi/Bluetooth functions are discovered,
  while missing 802.11 drivers and the absent USB host stack remain visibly
  unavailable instead of being reported as connected.
- The Browser is an Interface Form above HexaDisplay. Its current document
  engine accepts local `hexa://`, `data:text/html` and bounded `http://` or
  `https://` resources. It receives a requester-bound Network Handle only for
  non-Guest sessions when PIMP networking is enabled. Text entered without a
  URL scheme is encoded for DuckDuckGo's canonical non-JavaScript HTML endpoint
  at `https://duckduckgo.com/html/?q=...`. Navigation follows at most three
  redirects and rejects an HTTPS-to-HTTP downgrade. Search pages are projected
  into at most eight result titles and links. The allocation-free
  document core accepts at most 16 KiB, while the native HTTP client retains at
  most the first 14 KiB of a response body; a document contains at most 48
  nodes, 32 CSS rules, 16 scripts, 24 statements per script and 12 click
  handlers. It computes a bounded CSS subset for tag, class, id and inline
  rules, then runs a deterministic JavaScript subset for document title, node
  text, supported styles, visibility and local click handlers. It does not
  evaluate arbitrary ECMAScript or load external scripts, stylesheets, images
  or fonts, and it exposes no general Web APIs, cookies or storage. Media
  containers/codecs, audio/video output and GPU rendering are absent. A
  verified fetch of `www.youtube.com` may return HTML, but YouTube playback is
  not supported.
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

Form contents, relationships and PIMP state are deliberately backed by
fixed-capacity, in-memory tables at this stage. Account and desktop preference
mutations are durable through the separate state journal, but that journal is
not a substitute for the HexaFS block driver, persistent Form graph or FIN
index.

Alpha.12 includes the runtime-selectable 480p/720p/1080p empty-start desktop,
normal case-sensitive text, Notes, a richer graphical Terminal, six themes,
seven procedural wallpapers, four cursor themes, double-buffered presentation
with damage-region page synchronization, persistent 60/75/120/144 Hz software
pacing and optional VSync, a bounded native HTML/CSS/JavaScript Browser with
DuckDuckGo non-JavaScript HTML search, dual graphical and console login
selection, durable accounts/preferences, Ayo v3 artifact transactions, and
capability-gated native RTL8139/ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS networking.
The Diamond II build is
also exposed through `make run-alpha`, providing a runnable migration fallback
for networking, scheduling, ATA persistence and the games not yet redesigned
around v8 semantics.
