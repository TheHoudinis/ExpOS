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
  ASL v1 inventory -> exclusive driver claim
          |
          v
CfcFin -> CFC ownership/catalog (semantic core)
          |
          v
 FIN -> ExpFS current Form graph -> Dimension Binding
          |                    |
          v                    v
 CFC-scoped Handle <--- capability decision
          |
          v
 PIMP specification -> DIESE resolution
          |
          v
 ExpFS transaction -> alternating current-state snapshots
          |
          +-> checkpoint record/view semantics
          |
          v
 Form Execution Context -> hardware-timer-preempted CPL3 / per-Form CR3 / SMP AP job
```

## Approved target architecture (not yet complete)

Genesis constructs a Central Inflation Fabric (CFC) as the complete ExpOS
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
may not replace that baseline. The implemented suite is XChaCha20-Poly1305;
Basic uses Argon2id v1.3 with 64 MiB, three passes and one lane to wrap a random
32-byte key. Nonces bind that key to CFC identity, generation and slot domain.
Genesis v2 Paranoid uses six passes, requires active NX and SMEP, and omits the
network, Browser and ExpPython runtime surfaces. Encrypted manifest-v4 key
envelopes authenticate the complete redundant installation plan as associated
data, so offline policy expansion prevents successful key unwrap.
Alternating current-state commits and the eight-slot checkpoint ring are
authenticated-encrypted whenever the CFC has a storage key. The immutable
installation baseline uses the same snapshot schema in a separate, non-rotating
slot with its own authenticated-encryption domain.

The approved confinement/enforcement names are `ExpScope` for CFC/Dimension-
aware confinement, `ExpSeal` for monotonic capability reduction, and
`ExpBudget` for per-context resource enforcement. The semantic core now has a
deny-by-default fixed reachability scope, a broker-lifetime root-handle
issuance cutoff with strict attenuation, and fixed-capacity checked budgets.
The x86_64 execution path now applies address-space confinement and hardware
context switching to executable Form capsules. Enforcement at every
driver/storage boundary, durable execution-context seals, general executable
image loading and concurrent background dispatch have not landed. The
official configurable-installer label is **Architect**; “expert” is only a
legacy explanation. Native UEFI is the default boot path, while BIOS is
retained for compatibility, recovery, and development.

### ASL v1 and driver ownership

After the Form interrupt platform is established, the kernel inventories the
validated firmware framebuffer and PCI functions into 32 bounded ASL records.
Display, AHCI, NVMe, xHCI and Ethernet functions are classified without vendor
coupling. ExpDisplay, ExpStorage, ExpUSB and ExpNetwork can each claim a
matching PCI location once; a different owner cannot replace that claim. The
`asl` command reports boot-local IDs, locations, owners and MSI/MSI-X support
without disclosing BAR addresses. See `ASL_V1.md` for the exact first contract.

PCI MSI/MSI-X discovery, vectors `0x40..0x7f`, message construction and the
programming path have landed. Drivers currently reserve but do not arm those
messages because device-specific IDT completion handlers have not landed; NVMe,
AHCI, xHCI and RTL8139 therefore retain bounded polling.

The boot handoff carries the ACPI RSDP. The x86_64 SMP layer validates the
RSDT/XSDT and MADT, starts as many as seven application processors with
INIT/SIPI, and installs a per-CPU GDT, TSS, kernel interrupt stack and GS-based
execution record. The BSP keeps the PIT scheduler tick; APs use local-APIC
timers. Scheduler slots are independently reserved, allowing separate prepared
Form contexts and CR3 roots to execute concurrently. The bounded foreground
command `execute-parallel <Form-A> <Form-B>` submits both slices before joining
either and refuses to run without at least two online CPUs.

### Current enforcement and execution flow

- `ExpScope` snapshots the owning CFC's Form set when a context is constructed.
  Sharing a Dimension grants nothing: a target must be CFC-owned and explicitly
  added to the fixed allowlist before `authorize` succeeds.
- `ExpSeal` permanently closes new root-Handle issuance for one
  requester/Dimension in a broker lifetime. Delegation preserves CFC, target
  and Dimension, can only remove operation bits or shorten expiry, and parent
  revocation cascades. The current network right is the coarse `NETWORK` bit;
  `Connect`, `DNS`, and `RawPacket` have not yet been split into distinct bits.
- Core `ExpBudget` charges context-owned resource accounts before work and
  rejects overflow or a soft-limit breach atomically. Native scheduler slices
  charge `CpuTicks` delivered by the BSP PIT or AP local-APIC timer; code that
  never yields is stopped at the soft limit. Kernel controls separately enforce live Form
  bytes/operations, event watches, IPC reservations, and scratch pages. PIMP
  does not yet compile budget keys directly into every context's limits.
- `execute` requires an active target-scoped Execute Handle and admits a
  CFC/Dimension/FIN context. Kernel-native Forms retain trusted entrypoints.
  Persisted `executable` Forms receive a unique CR3 root with supervisor-only
  kernel mappings plus bounded user code/data/stack pages, enter CPL3 through
  `iretq`, and communicate through requester-bound Form ABI Handles at vector
  `0x80`. The initial capsule loader supports bounded log/exit programs and a
  non-yielding negative-control payload; general ELF/SDK image loading remains
  a later stage.
- The Network Driver Form uses bounded DHCP Discover/Offer/Request/ACK over
  the existing Ethernet/IPv4/UDP path. The accepted lease supplies address,
  netmask, gateway, DNS server, server identity and duration; `dhcp` performs a
  Handle-gated renewal. A conservative static fallback keeps recovery usable
  when no DHCP server answers.

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
7. ExpFS preflights and atomically publishes typed system-database records.
8. The kernel starts a graphical Session Manager and maps the selected identity
   to Operator, Power or Guest authority before opening the command environment.
   COM1, PS/2 and xHCI boot-HID input remain polling devices; the native Form
   platform separately installs per-CPU GDT/TSS/interrupt state, remaps the PIC,
   starts ACPI-described APs and programs PIT/local-APIC timers for CPL3
   preemption. Commands can
   create and inspect Forms, change lifecycle state, grant or revoke Handles,
   validate PIMP specifications, inspect system state, reboot, and shut down.
9. Bounded NVMe, AHCI SATA or ATA-PIO drivers load a dedicated ExpOS state image.
   The native ExpFS adapter recovers the newest valid of two CRC-protected CFC
   database snapshots and commits the inactive payload before its header. A
   snapshot contains arbitrary Form identity/content, revisions, Dimensions,
   relationships, PIMP network state, allocator state, accounts and desktop
   settings. The older account/preference slots are read-only migration input.
10. A fixed-capacity kernel-control service exposes typed tunables, readiness
    watches and resource ledgers to the console. DIESE checks the session's
    revocable bootstrap Handle before every mutation.
11. The semantic core models a CFC with a distinct `CfcFin`, required name,
    non-replaceable Primary Dimension, seven optional secondary Dimensions,
    sixteen owned Forms and thirty-two bindings. A four-CFC catalog rejects
    every cross-CFC Form/Dimension FIN reuse, including cross-kind collisions.
12. Capability brokers, Handles, and ExpFS transactions carry a CFC
    identity. ExpScope, ExpSeal, and ExpBudget provide bounded, allocation-free
    policy primitives; kernel integration remains partial.
13. Form-native execution contexts carry CFC, Dimension, FIN, address-space
    identity, a bounded Handle set, event queue, complete interrupt-frame CPU
    state, and ExpBudget. Executable Forms switch to per-Form page tables and
    CPL3, are sliced by per-CPU hardware timers, resume from saved registers,
    and retain ABI exit/fault results. Independent AP jobs permit concurrent
    contexts. `make ring3-check` proves four-CPU bring-up, two distinct AP slots
    dispatched before either completion, distinct CR3 roots, and an infinite
    loop stopped at its hardware-tick quota.
14. CFC-bound recovery metadata stores an installation-baseline descriptor
    outside an up-to-eight-entry rotating checkpoint ring. The native ExpFS
    adapter now persists eight complete CFC database checkpoints, rotates the
    oldest slot, lists retained state IDs and restores a selected checkpoint
    into the alternating current-state slots before reboot. A protected
    installation baseline is stored outside the ring and can only be copied
    back into current state; checkpoint operations cannot overwrite it.
    XChaCha20-Poly1305 authenticated encryption is applied to all recovery
    payloads for encrypted CFCs.

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
- ExpDisplay Portal v3 uses a Wayland-like ownership model without copying Wayland's
  Unix socket/file-descriptor ABI: clients own surfaces and Buffer Handles,
  mutate pending state, report surface-local damage, and publish atomically
  with `commit`. This is a Form-native compositor protocol, not Wayland
  compatibility. Damage is clipped to the surface and coalesced without
  allocation. Moves, resizes, visibility changes and destruction also retain
  the old output footprint so damage-driven composition cannot leave ghosts.
  Multiple commits before presentation retain their combined damage and
  request one callback carrying the newest commit sequence.
  `FrameDone` is emitted only after the compositor crosses the framebuffer
  presentation boundary; repeated callbacks and pure pointer motion coalesce,
  while a full queue can replace an obsolete motion sample with a key, button,
  focus or configure edge. Portal health exposes surface/event pressure,
  commits, frames, callbacks, coalescing, recovery and drop counters. Focus,
  configure, frame-complete, key and pointer events are routed back to the
  owning FIN. Native UEFI output consumes the validated GOP address, size,
  geometry, stride and RGB/BGR order without reprogramming a Bochs device.
  Composition happens in a bounded shadow scanout; only a completed normalized
  damage set is copied to GOP memory, so intermediate clearing and repaint
  steps cannot flicker on the physical display. Portal v3 promotes damage over
  two thirds of the visible frame to a sequential full copy and reconciles the
  complete shadow after 120 partial publications. It samples every published
  region through hardware readback; a mismatch triggers an immediate full
  shadow-to-scanout recovery instead of leaving corrupted pixels visible.
  BIOS fallback can program 640x480, 1280x720 or 1920x1080 XRGB scanout in
  QEMU standard VGA's 16 MiB linear framebuffer BAR. The bootstrap maps RAM
  and PCI windows below 4 GiB. ExpDisplay checks mapped bounds and geometry
  before the first framebuffer write. On the Bochs path, when the adapter accepts a virtual height of twice
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
  every application surface to its default geometry. Even with off-screen
  travel enabled, the compositor retains a useful titlebar grab area so a
  restored window cannot resemble a clipped or corrupted surface. Settings offers 256
  coordinated theme palettes: six named palettes (including Aurora and Rose)
  plus 250 bounded procedural palettes. It also provides 252 persisted wallpaper
  variants across seven procedural patterns (including Aurora and Mesh), 256
  accent colors, 256 backdrop tones, four cursor themes, five bitmap font
  faces, three independently rasterized font weights and the three display
  presets. Palette controls expose their stable numeric ID and a live swatch;
  keyboard coarse stepping jumps sixteen IDs at a time. Font selections apply
  globally without changing the fixed glyph
  advance. Window radius, border width and backdrop/titlebar opacity feed the
  actual compositor drawing, while titlebar height also changes drag/control hit-test
  geometry; cursor shadow is an independent low-cost rendering option. The
  taskbar can occupy any edge, use
  one of nine thicknesses, align running apps at start/center/end, auto-hide and
  reveal at that edge, blend translucently, show horizontal labels, and include
  RTC seconds plus date, active-app, Weather, presentation-rate, audio and
  time-zone widgets. Windows also supports persisted keyboard tiling into
  halves, thirds or quarters. The eighteen-category Settings UI adds coordinated whole-desktop
  Profiles, dedicated Accessibility, Terminal and Language pages, and a Menu
  page for list, grid, compact-grid and dashboard layout, density, scale,
  content visibility, categories and motion, and computes compact category/row
  viewports so the selected item remains visible at 480p. Appearance, Windows
  Taskbar, Menu, Date & time and Profiles expose 1,296 directly working selectable values.
  Profiles atomically apply Balanced, Compact, Focus, Accessible, Showcase,
  Touch-friendly, Night or Presentation combinations through the same persisted
  preference path. Accessibility also persists reduced transparency and an
  explicit focus ring. Its
  Display page also selects a 60, 75, 120 or 144 Hz compositor presentation
  target and optional VSync. These are software-pacing targets, not negotiated
  physical monitor modes. The Performance page independently controls window
  shadows, procedural wallpaper rendering and an Efficient/Responsive policy.
  Efficient mode paces every frame; Responsive mode bypasses software pacing
  only for damaged commits, while full redraws stay paced and VSync remains an
  independent presentation constraint. All three optional features persist and
  default off or Efficient so a fresh state starts on the least expensive path.
  Weather animation is capped at four damaged frames per second, and repeat
  refreshes reuse persisted coordinates instead of repeating geocoding.
  Weather network calls use an exact-host HTTPS allowlist, a requester-bound
  Handle, three-second refresh throttling and cached geocoding coordinates.
  The graphical Terminal keeps bounded scrollback and command history, exposes
  identity, system, display, network, CPU, application and ExpFS Form
  list/read/write commands, draws the same minimal two-eye `neofetch` art as the
  console, and provides window-layout recovery. Its face, weight, scale,
  foreground and background persist independently of the desktop font. English,
  Russian, Hebrew, German and Esperanto locale selections persist alongside
  these settings; the renderer supplies the required built-in glyphs and a
  bounded right-to-left presentation path for Hebrew.
  Each application receives a child Handle containing
  only Display and Input rights; the compositor checks it before visibility,
  geometry, commit or key routing. Ctrl+Shift+Esc opens a confirmation before
  safely leaving graphics and returning to the real command shell; plain Esc
  only cancels an editor or launcher, so GOP is not torn down by an accidental
  keypress. BIOS-only graphics exits
  retain a VGA mode 3 recovery path.
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
  requested modes, policy, pacing, portal pressure, copy cost and scanout
  recovery counters. `displaydebug on` overlays those live counters and
  `displayrepair` requests a full reconciliation without leaving the desktop.
  `stateinfo` reports the persistent
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
- Accounts and desktop preferences are typed records in the ExpFS current-state
  transaction. The older two-slot EXPOST03 area is read-only migration input;
  compatible state moves into ExpFS on its next mutation. Settings persist
  display mode, theme, wallpaper variant, cursor, accent,
  backdrop, pointer speed, presentation rate, VSync, shadows, wallpaper
  effects, presentation policy and desktop/connectivity flags. A tagged compact
  extension in the preference record persists and sanitizes 1,140
  accepted states for font face/weight; window radius, border, titlebar,
  opacity, off-screen allowance, snap and focus; taskbar edge, size, alignment,
  auto-hide, translucency, labels and clock precision. This
  count represents accepted selector states and both states of booleans, not
  1,140 independent rows. All 256 theme, accent and backdrop byte values are
  valid stable palette IDs, while wallpaper values are bounded to 0..251.
  Older compatible records receive
  conservative extension defaults; they default to 60 Hz with VSync enabled and
  use the low-cost renderer policy. Fresh state remains 480p/60 Hz with effects,
  translucency, animations, cursor shadow and off-screen travel disabled. Form
  records, Notes `.txt` content, Ayo application state and PIMP revisions share
  the durable ExpFS CFC snapshot and eight-checkpoint recovery ring. Encrypted
  Basic CFCs use their Argon2id-wrapped random key for XChaCha20-Poly1305
  snapshots; unencrypted Architect/development records retain CRC. The
  protected installation baseline is stored outside checkpoint rotation and
  uses its own disk/nonce domain.
- ExpStorage probes NVMe, AHCI SATA and legacy ATA-PIO in that order behind one
  512-byte sector API. The NVMe path constructs admin and I/O submission and
  completion queues; AHCI constructs command-list/FIS/table DMA state and uses
  DMA EXT plus cache flush. Both have fixed below-4-GiB DMA buffers and bounded
  completion polls. Hotplug, multiple-namespace selection, non-512-byte NVMe
  LBAs, IOMMU remapping and device identity UI are outside this first driver.
- ExpUSB owns one xHCI controller, fixed command/event/control/interrupt rings
  and up to four boot-protocol keyboard or mouse devices. It performs slot,
  address, descriptor, configuration and endpoint commands, then translates
  interrupt-IN reports into the same Form-native input events as PS/2. Hubs,
  mass storage, Bluetooth and arbitrary HID report descriptors are not claimed.
- Network is a Driver Form protected by requester-bound Network Handles and
  PIMP policy. Its current polling RTL8139 path implements Ethernet, ARP,
  DHCP-configured IPv4 with a conservative fallback, ICMP echo,
  checksum-validated UDP, DNS A lookup, one
  bounded synchronous TCP client and HTTP/1.0 GET over plain TCP or
  authenticated TLS 1.3. The TLS client requires hardware RDRAND entropy,
  sends SNI, verifies the requested hostname, certificate time validity from
  the RTC, the chain and signatures, and accepts a fixed AES-128-GCM-SHA256
  suite. TLS record and chain storage are fixed-size. Its Web PKI trust store
  is not a general operating-system CA bundle: GlobalSign Root R1 is the
  default anchor, DigiCert Global Root G2 is selected only for
  `duckduckgo.com`, and ISRG Root X1 only for `wikipedia.org` and their
  subdomains. IPv6, physical Wi-Fi, concurrent
  sockets, TCP servers and interrupt-driven I/O are explicitly future work.
- Connectivity settings target a distinct Radio FIN through a requester-bound
  Configure Handle. The manager keeps software policy, PCI/USB presence, driver
  readiness and connection state separate. Disabling Network is checked by the
  real packet authorization path. PCI Wi-Fi/Bluetooth functions are discovered,
  while missing 802.11 drivers and the absent USB host stack remain visibly
  unavailable instead of being reported as connected.
- The Browser is an Interface Form above ExpDisplay. Its current document
  engine accepts local `expos://`, `data:text/html` and bounded `http://` or
  `https://` resources. Its live DOM/style/script arena resides in a heap-backed
  bounded page container rather than inside the compositor stack. Navigation
  parses into a staged candidate and swaps it only after complete validation;
  malformed, over-budget or unsupported documents increment rejection
  diagnostics and leave the last valid page usable. This prevents parser/data
  failures from tearing down the desktop, but is not yet a claim that the
  renderer itself executes in Ring 3. The chrome maintains six fixed tab sessions, each with
  twelve history entries and a scroll position, plus eight in-session
  bookmarks and a case-insensitive find overlay with match highlighting. It
  receives a requester-bound Network Handle only for
  non-Guest sessions when PIMP networking is enabled. Address-bar text without
  a URL scheme is encoded for Wikipedia's compact
  REST search endpoint. Results are projected into at most eight verified
  article titles and links without loading scripts, trackers, or page assets.
  Navigation follows at most three redirects and rejects an HTTPS-to-HTTP
  downgrade. DuckDuckGo result wrappers are still unwrapped before the request;
  direct article URLs use Wikipedia's live HTTPS
  REST summary endpoint and render into a bounded scrollable document. The allocation-free
  document core accepts at most 16 KiB, while the native HTTP client retains at
  most the first 14 KiB of a response body; a document contains at most 48
  nodes, 32 CSS rules, 16 scripts, 24 statements per script and 12 click
  handlers. It decodes common HTML text entities, projects additional semantic
  text tags and unloaded image placeholders, and computes a bounded CSS subset
  for tag, class, id and inline rules, including border radius, maximum width
  and line height. It then runs a deterministic JavaScript subset for document title, node
  text, supported styles, visibility and local click handlers. A bounded
  twelve-resource manifest loads authenticated external stylesheets,
  deterministic scripts, a 64x64 BMP image cache and a PCM WAV media cache.
  The per-document Web bridge admits at most eight GET-style fetch, cookie, or
  local/session-storage set/get requests and partitions all state by origin;
  only local storage is durable in `BrowserData.state`. It does not evaluate
  arbitrary ECMAScript or expose a standards-complete Web API surface. Fonts,
  compressed images/audio, video decoding/output and GPU rasterization are
  absent. The
  browser chrome is Chromium-like; the engine is not Blink/V8 or Chromium
  extension compatible. A
  YouTube addresses open an immediate bounded compatibility notice; playback
  is not supported.
- ExpAudio is a bounded kernel service owned through ASL. Its first backend
  drives the Intel/QEMU-compatible AC'97 PCM-out bus-master interface with one
  below-4-GiB DMA descriptor, persistent volume/mute policy and a 48 kHz stereo
  signed-16 output format. RIFF/WAVE integer PCM inputs are validated and
  converted from 8-192 kHz, mono/stereo and 8/16/24/32-bit depth. Settings,
  Browser `<audio>` controls and graphical Terminal diagnostics share this
  service. MP3/AAC/Opus, mixing, capture, HDA/USB audio and video codecs are not
  implemented.
- Form ABI v1 freezes language-neutral 72-byte requests, 40-byte responses,
  call/status numbers, FIN caller identity, and explicit Handles for identity,
  IPC, display, input/events, time, storage, networking, browser navigation,
  and package transactions. Rust, Go, C, and Python contracts are tested. The
  native gate validates CFC, requester, target, Dimension and operation before
  dispatch. Ayo installs use it, and executable capsules cross it from CPL3
  through a fixed shared page. General image loading and buffer-grant formats
  beyond the bounded `LOG` grant remain pending.
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
is not the native persistent format. Desktop Ayo package mutations now cross a
native `PACKAGE_TRANSACTION` call gate with a requester-owned Form Handle and
persist their state in ExpFS.

The 32-bit alpha is quarantined under `legacy/alpha32`. Its code may be ported,
but its paths, owner/group modes, file descriptors, sudo-like ACL behavior and
process naming must not leak into the new public model.

Form contents, relationships, PIMP state, accounts and desktop preferences are
fixed-capacity records in the persistent ExpFS CFC snapshot. Mutations publish
transactionally with a journal generation; the recovery catalog retains eight
complete checkpoints. Encrypted CFCs protect these snapshots with the CFC's
unwrapped storage key; unencrypted Architect/development state retains CRC.

ExpOS v9 includes the runtime-selectable 480p/720p/1080p empty-start desktop,
normal case-sensitive text, Notes, a richer graphical Terminal with two-eye
`neofetch` and window recovery, 256 theme palettes, 252 wallpaper variants, 256
accent colors, 256 backdrop tones, four cursor themes, five font faces, three
real weights, all-edge taskbar and bounded
off-screen window controls, double-buffered presentation
  with ExpDisplay Portal v3 atomic surface commits, multi-region damage,
  presentation-bound frame completion, bounded event/damage coalescing,
  adaptive GOP synchronization, scanout readback/recovery and live diagnostics,
  persistent 60/75/120/144 Hz
software pacing, opt-in responsive damaged commits and optional VSync, a
bounded native HTML/CSS/JavaScript Browser with
  Wikipedia REST search and a live Wikipedia summary reader,
  dual graphical and console login
selection, durable accounts/preferences, Ayo v3 artifact transactions, and
capability-gated native RTL8139/ARP/IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS networking,
plus bounded Form-native tunable/event/resource controls inspired by—but not
compatible with—FreeBSD interfaces. The Diamond II build is
also exposed through `make run-alpha`, providing a runnable migration fallback
for networking, scheduling, ATA persistence and the games not yet redesigned
around v9 semantics.
