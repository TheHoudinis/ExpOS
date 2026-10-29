# ExpOS v9.8 “Populis” development scope

Populis is the local development line for lower idle overhead, safer installation
and broader VM hardware support. It builds on v9.4 Lose and does not change the
frozen Form ABI v1 or the additive streaming ABI v2. This checkout is not a
commit, tag or release.

## Transactional native packages and signed delivery

SDK EXF1 import scripts use `ayo native begin/write/commit` instead of publishing
every `writehex` chunk as a Form revision. Ayo holds one bounded artifact
privately, requires contiguous offsets, verifies its declared length and
SHA-256, parses the complete EXF1 container, and only then performs the atomic
ExpFS executable-Form replacement. Interrupted or invalid staging is discarded,
so an already installed executable remains intact.

`ayo registry install <https-manifest> <ed25519-public-key-hex>` now adds the
automatic network path. The small `EXPOS-AYO-REGISTRY-V1` envelope is verified
against the explicitly pinned Ed25519 key, its EXF1 URL must be HTTPS and
same-origin with the manifest, the body must match the signed length and
SHA-256, and installation reuses the same transactional EXF1 commit. Guest
sessions cannot install. Unit tests cover a valid signature, altered metadata
and a wrong key. A public registry endpoint, production signing key and live
Internet end-to-end registry run are not supplied by this repository.

## Performance work

- AC'97 completion uses acknowledged PCI interrupts rather than a 200 Hz
  bus-master status poll. An atomic event handoff keeps the IRQ handler out of
  the audio lock.
- Both the shell and desktop arm PIT channel 0 in one-shot mode while idle. The
  default maximum sleep is 50 ms; an active background Form uses a bounded 1 ms
  quantum. `sti; hlt; cli` admits scheduler and device interrupts without
  exposing ordinary kernel lock sections.
- The periodic 1 kHz PIT is restored before Ring 3 execution, so the frozen
  preemption/accounting contract remains 1 kHz even though idle wakeups are no
  longer forced every millisecond.
- PCM resampling advances an incremental source phase, removing per-frame
  multiplication, division and modulo while preserving interpolation.
- Opaque framebuffer fills and copies now use aligned `rep stosq`/`rep movsq`
  64-bit integer string operations with a bounded dword head/tail. This doubles
  the hot-path transfer width without touching SIMD/FPU state that is not yet
  part of the Ring 3 context contract.
- GOP partial-damage readback is sampled once every 16 publications instead of
  serializing every cursor-sized update on uncached MMIO. Full publications are
  still always checked, periodic full reconciliation remains active, and a
  mismatch still triggers a full-copy recovery.

`make tickless-check` requires the one-shot runtime marker and a clean QEMU
shutdown. The e1000/e1000e test also proves the idle window admits a real device
completion interrupt.

## Genesis multi-disk identity gate

Genesis builds a bounded inventory of up to eight usable targets and displays
model, serial, backend, capacity, controller, removable-media state and a stable
FNV-1a identity fingerprint before configuration. The operator selects an
inventory index and must retype that target's exact serial. The later `ERASE`
confirmation remains separate, and no write occurs during selection.

The current inventory combines the storage families implemented by this tree:
one exposed NVMe namespace, one exposed AHCI target, transitional VirtIO block,
and both primary-channel ATA positions. The QEMU Genesis gate attaches a decoy
as ATA master and the intended target as ATA slave, selects the target by index
and serial, then verifies the decoy's first 4 KiB is unchanged before booting
the installed target. Controllers exposing multiple AHCI ports or multiple
NVMe namespaces still need family-specific enumeration expansion.

This is VM safety evidence only. It does not approve destructive installation
on a physical disk.

Genesis now also records three install-time display choices in the authenticated
redundant plan: Automatic/VirtIO-GPU/compatible graphics selection, 480p/720p/
1080p initial resolution, and Efficient/Balanced/Responsive rendering policy.
On the first installed boot these seed normal persistent desktop preferences;
later user changes remain authoritative and are not overwritten on every boot.

## Operational VM drivers

- **Intel e1000/e1000e:** bounded controller reset, 32-entry RX and 8-entry TX
  descriptor rings, static DMA buffers below 4 GiB, asynchronous transmit,
  status-gated descriptor reuse, and I/O-APIC-routed INTx acknowledgement.
  QEMU 82540EM (`100e`) and 82574L (`10d3`) each complete DHCP, two ICMP
  exchanges and at least one delivered completion interrupt.
- **VirtIO PCI:** one bounded transitional/legacy PCI transport owns status
  negotiation, feature acceptance, queue sizing, split-ring layout, descriptor
  submission, notification and used-ring consumption. `virtio-net` completes
  DHCP/ping and `virtio-block` passes a two-boot write/read persistence test.
  Modern-only VirtIO 1.x capability transport (`1041`/`1042`) remains
  probe-only and is not described as operational.
- **VirtIO GPU:** the modern VirtIO 1.x PCI capability transport is operational
  for QEMU's `1050` display device. It negotiates `VIRTIO_F_VERSION_1`, programs
  a bounded split control queue, creates and backs one B8G8R8X8 2D resource,
  selects scanout 0, then transfers and flushes bounded damage. Composition and
  rasterization remain CPU-side; this is accelerated device-backed 2D scanout,
  not VirGL/Venus/OpenGL or physical-GPU support. `make gpu-check` verifies a
  visibly composed screenshot plus 480p, 720p and 1080p command completion.
  Populis keeps the CPU rasterizer but removes avoidable work around it: opaque
  fills/copies use widened string instructions, alpha surfaces blend in clipped
  RAM scanlines, and a cached wallpaper base is restored instead of regenerating
  procedural effects on every full composition. Unchanged backgrounds publish
  only previous/current scene footprints. The GPU path chooses exact separated
  transfers when a bounding rectangle would exceed twice their pixel count and
  performs one bounding flush while holding the device lock once.
- **Compositor scheduling:** queued input is processed in order while at most
  seven obsolete intermediate visual states are coalesced into the newest
  composition. At least every eighth request renders, so a continuously busy
  device cannot starve scanout or frame callbacks. Idle cadence slots are marked
  idle before rasterization; pacing uses bounded one-shot PIT waits plus a short
  spin tail instead of a multi-million-iteration busy wait.
- **Intel HDA:** controller reset, codec/AFG/DAC/pin discovery, immediate
  command verbs, 48 kHz signed-16 stereo setup, a bounded 32-entry BDL and
  completion/error acknowledgement. QEMU captures non-empty WAV output and
  reports two delivered stream interrupts. AC'97 remains the fallback.
- **VirtIO Sound and MP3 media:** modern PCI capability transport discovers a
  compatible output stream, configures 48 kHz signed-16 stereo through the
  specified SET_PARAMS/PREPARE/START lifecycle, and advances output from TX
  used-ring completions. A safe no-std decoder fills only sixteen MPEG Layer
  III frames per refill, keeping one active and one pending device buffer rather
  than expanding a whole song to PCM. The MP3 binaries are excluded from the
  repository and ISO. `ayo music install <imtq|dash> <web-url>` uses retryable
  bounded Range requests, pinned size/SHA-256 verification, inactive-slot
  writes, full disk read-back and atomic redundant catalog publication on the
  unpartitioned VM state disk. `play` refuses before installation. `make
  modern-audio-check` serves the external originals, proves the refusal,
  installs and streams both tracks, captures non-empty stereo PCM, then reboots
  with the server stopped and plays both from persistent storage.

ASL claims only operational backends and continues to label unsupported modern
VirtIO net/block capability-only devices as probe-only; GPU and Sound now have
bounded operational modern transports. None of these QEMU results is
physical-device compatibility evidence.

## Validation status

The focused development gate is:

```sh
cargo test -p expos-kernel --lib
cargo clippy --release -p expos-kernel --target x86_64-unknown-none -- -D warnings
make gpu-check
make tickless-check
make e1000-network-check
make virtio-network-check
make modern-hardware-check
make hda-check
make audio-check
make modern-audio-check
make network-check
make genesis-check
git diff --check
```

`make gpu-check` now enforces a composed nonblank screenshot and performance
bounds for the deterministic input burst. On the current VM run it produced 28
presented frames for 216 requested visual states, coalesced 188 obsolete
compositions, copied 8,238,148 pixels, and recorded 86 missed slots. The
pre-change log for the same workload recorded 216 frames, about 65.9 million
copied pixels and 28,346 missed slots; timings remain host-sensitive, so the test uses
looser regression ceilings rather than requiring those exact values.

Passing these gates demonstrates only the implemented local QEMU paths. It is
not permission to commit, push, tag or publish a release.
