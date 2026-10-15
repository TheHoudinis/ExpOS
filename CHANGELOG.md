# ExpOS changelog

## 9.4.0 "Lose" - 2026-10-15

This release hardens the v9.3 Loop architecture without changing the frozen
Form ABI v1 layout or call numbers.

### Added

- Chunked Form blobs backed by 4 KiB chunks, with a 32 KiB per-Form quota and
  a 48 KiB per-CFC live-content quota.
- A separate capability-checked streaming ABI v2 for large Form contents.
- EXF1 v2 multi-page executables with eight code pages, six data pages, a
  private 64 KiB RW/NX heap, four stack pages and four guard regions.
- Firmware-map physical-frame allocation and lease tracking for dynamically
  mapped Form pages.
- Background Form lifecycle control through `formctl`: start, pause, resume,
  wait, terminate and restart, with bounded asynchronous IPC and event wakeups.
- C and Rust SDK output for EXF1 v2 and bounded operation-budget escalation for
  larger SDK imports.
- A 32-descriptor AC'97 PCM-out DMA ring, active/pending sample banks and one
  bounded queued playback request.
- PCM WAV conversion for mono or stereo 8/16/24/32-bit integer audio from
  8-192 kHz, with linear interpolation to 48 kHz stereo signed-16 output.

### Fixed

- Failed EXF1 v2 preparation now releases allocated frames and the reserved
  runtime slot instead of leaking an active address space.
- The physical-frame allocator now distinguishes dynamic leases from firmware
  and kernel reservations, preventing reserved pages from being released by a
  bad caller.
- AC'97 initialization uses bounded reset polling and rolls back its ASL claim
  when controller activation fails.
- WAV parsing now rejects inconsistent RIFF sizes, malformed or duplicate
  chunks, invalid byte rates/block alignment and partial sample frames.
- AC'97 playback no longer truncates long buffers into a single 16-bit DMA
  descriptor, and volume attenuation is no longer applied twice.
- FIFO errors and unexpected controller halts are recorded and surfaced in
  audio diagnostics; queued playback is advanced from the desktop service loop.

### Compatibility and known limits

- Form ABI v1 remains frozen at its existing request/response layouts and call
  numbers. Large transfers use ABI v2 rather than widening ABI v1.
- The supported audio backend remains Intel/QEMU-compatible AC'97 playback.
  There is no HDA, USB audio, capture, compressed codec, video or software
  mixer support. Playback servicing is bounded polling, not an armed audio IRQ.
- Physical hardware validation is still required before claiming support for a
  particular machine. QEMU evidence does not prove physical-device behavior.
- Genesis still lacks a physical-disk identity picker, the network service has
  no hardware IOMMU, and Ayo has no public registry in this release.

See [docs/LOSE_V9_4.md](docs/LOSE_V9_4.md) for the release scope, validation
commands and remaining boundaries.
