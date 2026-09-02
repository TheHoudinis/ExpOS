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
8. The kernel starts an interactive Operator command environment. Polling COM1
   and PS/2 input work before the interrupt subsystem is ported. Commands can
   create and inspect Forms, change lifecycle state, grant or revoke Handles,
   validate PIMP specifications, inspect system state, reboot, and shut down.

## Trust boundaries

- Exact resolution uses FIN. Human-readable names are secondary and scoped by
  a Dimension binding.
- Operator, Power and Guest are authority inputs to capability decisions; they
  are not Unix UID aliases.
- Handles carry requester and target FINs, allowed operations, Dimension,
  expiry and revocation state. No file descriptor abstraction appears in the
  core.
- Relationships are typed, FIN-to-FIN and optionally Dimension-scoped; package
  dependencies use the same model instead of paths.
- The Go ayo catalog maps verified registry metadata into Package Forms. A
  dependency plan is validated and committed as one state transaction rather
  than downloading archives into paths.
- HexaDisplay uses a Wayland-like ownership model without copying Wayland's
  Unix socket/file-descriptor ABI: clients own surfaces and Buffer Handles,
  mutate pending state, report damage, and publish atomically with `commit`.
  Focus, configure, frame-complete and key events are routed back to the owning
  FIN. The alpha renderer targets the mapped Bochs/QEMU XRGB framebuffer.
- The desktop creates separate Browser, Terminal, Forms, Packages, Settings and
  System surfaces, plus Root, panel and launcher surfaces. Visibility changes,
  focus transitions and movement are published through HexaDisplay commits.
  Leaving graphics restores the VGA mode 3 register set before the kernel
  redraws its text console.
- The Browser is an Interface Form above HexaDisplay. Its current document
  engine intentionally accepts local `hexa://` and `data:text/html` resources;
  external HTTPS, CSS and JavaScript are not claimed while the v8 network and
  isolation layers remain unbound.
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

Alpha.3 ports the broad hardware, utility and Form-content command layer. The
Diamond II build is also exposed through `make run-alpha`, providing a runnable
migration fallback for games, networking, scheduling, ATA persistence and VBE
while their public interfaces are redesigned around v8 semantics.
