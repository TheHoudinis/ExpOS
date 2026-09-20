# Architecture status

The rebuild is split into a small trusted semantic core and platform adapters.
Host tests exercise the same `no_std` core that is linked into the kernel.

```text
native UEFI / BIOS compatibility fallback
          |
          v
  x86_64 bootstrap kernel
          |
          v
CfcFin -> CFC ownership/catalog (semantic core)
          |
          v
 FIN -> Form Registry -> Dimension Binding (runtime integration partial)
          |                    |
          v                    v
 CFC-scoped Handle <--- capability decision
          |
          v
 PIMP specification -> DIESE resolution
          |
          v
 ExpFS transaction + journal sequence
```

## Approved target architecture (not yet complete)

Genesis constructs a Central Finite Curve (CFC) as the complete ExpOS
environment. Every CFC has its own typed FIN, a required nonempty name, and
exactly one Primary Dimension. Forms, Dimensions, relationships,
identity/policy records, capabilities, ExpFS state, keys, checkpoints, and
storage extents have one exclusive CFC owner. Cross-CFC sharing is forbidden;
an explicit transfer creates a new destination-owned entity.

Each encrypted CFC has a random storage key. Argon2id derives a key-encryption
key (KEK) from the Operator password to wrap and unlock that storage key; it
does not use the password directly as the data key. Persistent state of an
encrypted CFC requires AEAD. Every CFC retains eight rotating checkpoints plus
a protected immutable installation baseline; encrypted CFC recovery state is
authenticated by its storage protection. Normal writes, rotation, and restore
may not replace that baseline. Exact AEAD choice, Argon2id parameters,
key-envelope layout, nonce construction, checkpoint format, and
crash-consistency protocol remain to be specified and implemented.

The approved confinement/enforcement names are `ExpScope` for CFC/Dimension-
aware confinement, `ExpSeal` for monotonic capability reduction, and
`ExpBudget` for per-context resource enforcement. The semantic core now has a
deny-by-default fixed reachability scope, a broker-lifetime root-handle
issuance cutoff with strict attenuation, and fixed-capacity checked budgets.
Complete address-space/execution confinement, enforcement at every
driver/storage boundary, durable execution-context seals, and
scheduler/resource-owner accounting have not landed. The
official configurable-installer label is **Architect**; “expert” is only a
legacy explanation. Native UEFI is the default boot path, while BIOS is
retained for compatibility, recovery, and development.

## Implemented vertical slice

1. The loader validates long-mode support and enters an identity-mapped x86_64
   kernel with a 1 MiB bootstrap stack.
2. The kernel validates the native UEFI or Multiboot2 handoff, initializes polling serial and VGA output,
   and invokes `expos_core::bootstrap_demo`.
3. The demo creates a Root Form and Stable Dimension with independent FINs.
4. A binding makes the Root Form visible in Stable at revision 1.
5. Typed PIMP settings request service mode, restricted networking and
   isolation. DIESE resolves them using explicit scope precedence and rejects
   same-scope conflicts with a diagnostic.
6. The capability broker returns a time-limited, Dimension-scoped Form Handle
   that authorizes only read and execution.
7. ExpFS preflights and atomically publishes a journaled metadata transaction.
8. The kernel starts a graphical Session Manager and maps the selected identity
   to Operator, Power or Guest authority before opening the command environment.
   Polling COM1, PS/2 keyboard and PS/2 mouse input work before the interrupt
   subsystem is ported. Commands can
   create and inspect Forms, change lifecycle state, grant or revoke Handles,
   validate PIMP specifications, inspect system state, reboot, and shut down.
9. A bounded primary-master ATA PIO driver loads a dedicated ExpOS state image.
   Accounts and desktop preferences are recovered from the newest valid of two
   CRC-protected journal slots and mutations alternate slots by generation.
10. A fixed-capacity kernel-control service exposes typed tunables, readiness
    watches and resource ledgers to the console. DIESE checks the session's
    revocable bootstrap Handle before every mutation.
11. The semantic core models a CFC with a distinct `CfcFin`, required name,
    non-replaceable Primary Dimension, seven optional secondary Dimensions,
    sixteen owned Forms and thirty-two bindings. A four-CFC catalog rejects
    every cross-CFC Form/Dimension FIN reuse, including cross-kind collisions.
12. Capability brokers, Handles, and in-memory ExpFS transactions carry a CFC
    identity. ExpScope, ExpSeal, and ExpBudget provide bounded, allocation-free
    policy primitives; kernel integration remains partial.
13. CFC-bound recovery metadata stores an installation-baseline descriptor
    outside an up-to-eight-entry rotating checkpoint ring. Publication and
    batch validation are atomic and restore selection is non-destructive. This
    code does not capture, protect, encrypt, authenticate, persist, or restore
    disk contents.

## Trust boundaries

- Exact resolution uses FIN. Human-readable names are secondary and scoped by
  a Dimension binding.
- Operator, Power and Guest are authority inputs to capability decisions; they
  are not Unix UID aliases.
- Handles carry owning CFC, requester and target FINs, allowed operations, Dimension,
  expiry and revocation state. No file descriptor abstraction appears in the
  core. A Handle can derive only narrower children: delegation cannot add an
  operation, extend expiry, or change target/Dimension, and revoking a parent
  recursively revokes its descendants.
  ExpSeal can monotonically close ambient root-handle issuance for one
  requester/Dimension during a broker's lifetime while allowing already issued
  authority to be delegated only through strict attenuation checks. Complete
  enforcement still requires a durable per-context broker/seal registry and
  every privileged kernel boundary to consume these CFC-scoped Handles.
- Relationships are typed, FIN-to-FIN and optionally Dimension-scoped; package
  dependencies use the same model instead of paths.
- Ayo v3 maps verified registry metadata into Package Forms and materializes
  raw, tar or tar.gz artifacts beneath an explicit user-owned root. Downloads,
  extraction, owned-file receipts, state publication, uninstall and recovery
  share an atomic transaction; scripts, symlinks and path traversal are denied.
- The kernel-control service translates three established FreeBSD design ideas
  into original Form-native data structures; it does not reuse FreeBSD source,
  system-call numbers, layouts or ABI. The sysctl-style registry contains nine
  typed nodes with explicit read-only or Operator-write access and validates
  boolean and bounded integer input. The kqueue-style service has fixed rings
  for 16 watches and 16 ready records, `(identifier, filter)` identity,
  monotonic event sequences, configurable 1-16 dispatch batches and optional
  coalescing. Signal, one-shot/periodic timer and resource-denial filters are
  implemented. Missed periodic expirations accumulate rather than silently
  disappearing. The ExpBudget ledger, derived from an rlimit-style concept,
  tracks event watches, IPC bytes,
  scratch pages, Form operations and live Form content bytes; each record enforces
  `used <= soft <= hard <= ceiling`, charges against the soft limit, records
  denials and can raise resource-readiness events. Event watches use this ledger
  internally. The shell charges every Form mutation attempt and checks content
  growth before writes/copies, releasing bytes on shrink/reclamation. These
  owned counters cannot be forged through manual charge/release commands.
  IPC bytes and scratch pages remain explicit runtime-local reservations.
  Reads are available to every authenticated authority. Tunable and limit
  changes require a Configure-capable bootstrap Handle issued only to Operator;
  watch/signal/poll and charge/release require its Execute right. Revoking that
  Handle removes mutation access.
  The tables fail closed at capacity and remain runtime-local. They are not a
  scheduler, an interrupt notification backend or a FreeBSD compatibility
  subsystem. It now uses the ExpBudget name, but remains one runtime-local
  command context rather than scheduler-wide per-Form enforcement.
- ExpDisplay uses a Wayland-like ownership model without copying Wayland's
  Unix socket/file-descriptor ABI: clients own surfaces and Buffer Handles,
  mutate pending state, report surface-local damage, and publish atomically
  with `commit`. This is a Form-native compositor protocol, not Wayland
  compatibility. Damage is clipped to the surface and coalesced without
  allocation. Moves, resizes, visibility changes and destruction also retain
  the old output footprint so damage-driven composition cannot leave ghosts.
  Multiple commits before presentation retain their combined damage and
  request one callback carrying the newest commit sequence.
  `FrameDone` is emitted only after the compositor crosses the framebuffer
  presentation boundary; queue pressure defers rather than drops it. Focus,
  configure, frame-complete, key and pointer events are routed back to the
  owning FIN. The software renderer can program 640x480, 1280x720 or 1920x1080 XRGB
  scanout in QEMU standard VGA's 16 MiB linear framebuffer BAR. The bootstrap
  maps RAM and PCI windows below 4 GiB. ExpDisplay discovers the firmware-assigned
  VGA BAR and checks its mapped bounds and the selected geometry, stride and
  double-buffer byte count against the reported video memory before the
  first framebuffer write. When the adapter accepts a virtual height of twice
  the visible height, rendering targets the hidden page and presentation flips
  the VBE Y offset. Every flip is read back from hardware; if the adapter
  rejects it, the completed damage is copied to the prior visible page and
  page flipping is disabled. Before synchronization, scanout damage is clipped
  in a fixed 32-region stack buffer. A bounding merge is used only when it does
  not copy more pixels than the original regions; overflow collapses to one
  safe bounding rectangle instead of losing pixels or allocating. After a
  successful flip, only normalized damage
  is copied to the newly hidden page, keeping the two pages coherent without a
  full-screen copy for cursor and terminal updates. Submitted/copy region and
  pixel totals, collapse count and flip failures remain observable. Optional VSync performs a bounded
  legacy-VGA retrace wait; failed waits increment a diagnostic counter instead
  of blocking forever. Fresh state requests 480p and 60 Hz. The boot chooser
  and graphical login explicitly present their back buffers before waiting for
  input, and a rejected saved mode is retried at 480p before console fallback.
  Checksummed but out-of-range display preferences fail down to their lowest
  supported values rather than being clamped upward.
- The desktop creates separate Browser, Terminal, Forms, Packages,
  Settings, System, Games and Notes surfaces, plus Root, taskbar and launcher
  surfaces. Its flat dark shell provides a compact application menu and
  edge-configurable taskbar without copied third-party assets, promotional copy
  or instruction footers. It starts with no open or pinned apps and supports
  focus, dragging, minimize, maximize, close and reopen. Windows can remain
  contained or move a selected distance beyond the work area while retaining a
  bounded recovery region; optional snapping and three focus policies alter the
  real geometry/input path. `windowreset` in the graphical Terminal restores
  every application surface to its default geometry. Settings offers six
  renderer-defined themes (including Aurora and Rose), seven procedural
  wallpapers (including Aurora and Mesh), four cursor themes, five bitmap font
  faces, three independently rasterized font weights and the three display
  presets. Font selections apply globally without changing the fixed glyph
  advance. Window radius, border width and backdrop/titlebar opacity feed the
  actual compositor drawing, while titlebar height also changes drag/control hit-test
  geometry; cursor shadow is an independent low-cost rendering option. The
  taskbar can occupy any edge, use
  one of nine thicknesses, align running apps at start/center/end, auto-hide and
  reveal at that edge, blend translucently, show horizontal labels, and include
  RTC seconds. The eleven-category Settings UI computes compact category/row
  viewports so the selected item remains visible at 480p. Appearance, Windows
  and Taskbar expose 106 directly working selectable values. Its
  Display page also selects a 60, 75, 120 or 144 Hz compositor presentation
  target and optional VSync. These are software-pacing targets, not negotiated
  physical monitor modes. The Performance page independently controls window
  shadows, procedural wallpaper rendering and an Efficient/Responsive policy.
  Efficient mode paces every frame; Responsive mode bypasses software pacing
  only for damaged commits, while full redraws stay paced and VSync remains an
  independent presentation constraint. All three optional features persist and
  default off or Efficient so a fresh state starts on the least expensive path.
  The graphical Terminal keeps bounded scrollback and command history, exposes
  identity, system, display, network and application commands, draws the same
  two-eye `neofetch` art as the console, and provides window-layout recovery.
  Each application receives a child Handle containing
  only Display and Input rights; the compositor checks it before visibility,
  geometry, commit or key routing. Leaving graphics restores the VGA mode 3
  register set before the kernel redraws its text console.
- The PS/2 adapter enables the auxiliary device, validates ACKs and decodes
  synchronized three-byte packets. The compositor clamps a save-under cursor,
  hit-tests the topmost visible surface and checks its Input Handle before
  routing motion or button events. Up to sixteen consecutive pure-motion
  packets are folded into one saturating delta before a repaint. The first key
  or button transition is retained for the next poll, so batching cannot erase
  an input edge. The buffer protocol represents XRGB8888,
  ARGB8888 and RGB565 clients. Solid fills use clipped row writes; gradients,
  alpha blending, rounded rectangles and Bresenham lines are native primitives;
  cursor movement repaints only the cursor bounds; games repaint only the active window.
- Presentation pacing uses a calibrated TSC frequency when CPUID supplies one
  and a conservative fallback otherwise. Fractional frame periods are carried
  without cumulative integer drift, clock discontinuities are recoverable, and
  the graphical Terminal's `display` command reports pacing misses. The console
  `displayinfo` command reports page-flip/frame/retrace-timeout state, the
  graphical System surface reports frame count, and `timers` identifies the
  clock source. `displaydiag` exposes adapter identity, requested/active mode,
  memory bounds, presentation state, normalized-damage counters and flip
  failures; the graphical Terminal's `display` command exposes its active and
  requested modes, policy, pacing and scanout counters. `stateinfo` reports the persistent
  display selection; `diag` combines these with clock and network status.
  Operator-only `safevideo`/`displayreset` persists 480p, 60 Hz and VSync on as
  a console recovery path. A boot/login frame that cannot be confirmed visible
  falls back to the console, and an active desktop returns to the command
  environment instead of waiting behind a black scanout. If persistence fails,
  the sanitized values still
  replace the in-memory preferences for the rest of that boot.
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
- The state disk is a compact versioned store, not ExpFS. Two 2 KiB slots at
  fixed LBAs hold accounts and desktop preferences. Each commit writes the
  inactive slot with a monotonically advancing generation, format/version
  fields and CRC-32 over its header and payload; boot selects the newest valid
  slot, preserving the previous generation across an interrupted or corrupt
  write. Settings persist display mode, theme, wallpaper, cursor, accent,
  backdrop, pointer speed, presentation rate, VSync, shadows, wallpaper
  effects, presentation policy and desktop/connectivity flags. A tagged compact
  extension in the existing preference reservation persists and sanitizes 112
  accepted states for font face/weight; window radius, border, titlebar,
  opacity, off-screen allowance, snap and focus; taskbar edge, size, alignment,
  auto-hide, translucency, labels and clock precision; plus 28 reserved
  interaction states that are stored but not claimed as working controls. This
  count represents accepted selector states and both states of booleans, not
  112 independent rows. Older compatible records receive
  conservative extension defaults; they default to 60 Hz with VSync enabled and
  use the low-cost renderer policy. Fresh state remains 480p/60 Hz with effects,
  translucency, animations, cursor shadow and off-screen travel disabled. Form
  records, notes and PIMP revisions are outside this store and remain volatile.
  This current CRC journal is plaintext and is not the approved per-CFC AEAD
  store. It has neither an Argon2id-wrapped random storage key nor the target
  eight-checkpoint ring and protected installation baseline.
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
- The Browser is an Interface Form above ExpDisplay. Its current document
  engine accepts local `expos://`, `data:text/html` and bounded `http://` or
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
- ExpFS transaction commit validates all staged records and capacity before
  publishing any record under a single journal sequence.

## Transitional boundaries

The native UEFI path now loads the kernel directly, reserves its memory,
passes firmware memory-map/GOP metadata, and exits boot services. The older
GRUB/Multiboot2 path from expodOS is intentionally retained as a BIOS
compatibility, recovery, and development fallback. Native UEFI is the approved
Genesis/default path; see `BOOT.md` for hardware limits.
The `ayo` JSON store remains a
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
not a substitute for the ExpFS block driver, persistent Form graph or FIN
index.

Alpha.12 includes the runtime-selectable 480p/720p/1080p empty-start desktop,
normal case-sensitive text, Notes, a richer graphical Terminal with two-eye
`neofetch` and window recovery, six themes, seven procedural wallpapers, four
cursor themes, five font faces, three real weights, all-edge taskbar and bounded
off-screen window controls, double-buffered presentation
with atomic surface commits, presentation-bound frame completion, bounded
damage coalescing and page synchronization, persistent 60/75/120/144 Hz
software pacing, opt-in responsive damaged commits and optional VSync, a
bounded native HTML/CSS/JavaScript Browser with
DuckDuckGo non-JavaScript HTML search, dual graphical and console login
selection, durable accounts/preferences, Ayo v3 artifact transactions, and
capability-gated native RTL8139/ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS networking,
plus bounded Form-native tunable/event/resource controls inspired by—but not
compatible with—FreeBSD interfaces. The Diamond II build is
also exposed through `make run-alpha`, providing a runnable migration fallback
for networking, scheduling, ATA persistence and the games not yet redesigned
around v8 semantics.
