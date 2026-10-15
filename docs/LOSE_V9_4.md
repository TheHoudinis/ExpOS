# ExpOS v9.4 "Lose"

Lose is the stable-release hardening line built on v9.3 Loop. It preserves the
Form-native architecture and frozen Form ABI v1 while making larger Forms,
multi-page executables, background execution and AC'97 playback substantially
more robust.

## Release scope

### Storage and ABI

- Native Form content is stored as lazily allocated 4 KiB chunks.
- Each Form is limited to 32 KiB of live content and each CFC to 48 KiB.
- Complete replacements are revision-checked transactions: quota, expected
  revision and storage capacity are validated before publication.
- Form ABI v1 remains byte-for-byte frozen. Streaming ABI v2 is a separate,
  capability-checked contract with bounded chunks and explicit offsets.

### Native execution and memory

- EXF1 v2 maps up to eight code pages and six data pages, a private 64 KiB
  RW/NX heap, four stack pages and four unmapped guard regions.
- A firmware-map allocator owns physical-frame state independently of virtual
  mappings. Dynamic leases cannot release kernel or firmware reservations.
- Code pages are writable only while populated, then sealed executable and
  read-only before Ring 3 entry.
- Failed preparation unwinds every allocated frame and makes its runtime slot
  reusable.

### Background Forms

- `formctl` provides start, pause, resume, wait, terminate and restart.
- Requests use bounded asynchronous IPC queues and readiness events.
- Background dispatch occurs only during shell idle time and remains subject to
  hardware-tick ExpBudget preemption.

### Audio

- The ASL-owned AC'97 backend uses a 32-descriptor 48 kHz stereo signed-16 DMA
  ring with separate active and pending banks.
- One request may play while one is queued. A third request fails explicitly
  instead of overwriting live DMA storage.
- RIFF/WAVE parsing validates declared lengths, padding, chunk uniqueness,
  format metadata, frame alignment and bounded output size.
- PCM mono/stereo 8/16/24/32-bit integer input from 8-192 kHz is converted with
  linear interpolation. Master mute and PCM attenuation are applied once.
- Reset polling is bounded. FIFO errors, underruns, unexpected halts, completed
  buffers, queued requests and recoveries are reported by audio diagnostics.

## Validation contract

The supported release gate is:

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --release -p expos-kernel --target x86_64-unknown-none -- -D warnings
make all
```

`make all` covers the BIOS smoke path, display and audio paths, sessions,
networking, persistence and large files, modern-hardware discovery, SDKs,
Python budgets, Ring 3 execution, native UEFI and single-user boot, visible
startup, isolation probes, native EXF1 images, deterministic parser mutation
and editor workflows. The audio gate records QEMU AC'97 output to a WAV file,
crosses a DMA-descriptor boundary and rejects logged audio faults.

Passing these checks is evidence for the repository's QEMU-supported build. It
does not constitute physical-device, arbitrary-web, public-registry or release
publication proof.

## Supported boundaries

- Audio is playback-only AC'97. HDA, USB audio, capture, compressed formats,
  video and multi-client software mixing are not implemented.
- The AC'97 service is polled at a bounded cadence; an interrupt-driven handler
  is not armed.
- The network worker has no hardware IOMMU. Ring 0 validates DMA pages, limits
  registers and revokes bus mastering before releasing the worker arena.
- Genesis physical installation still requires a trustworthy disk identity
  picker before it can be recommended for unattended physical use.
- Browser support is a bounded native HTML/CSS/script subset, not Chromium or a
  general Web API. Unsupported or over-budget content must retain the last
  valid document.
- Ayo supports built-in, local and pinned-key HTTPS catalogs, but this release
  does not claim a public package registry.

## Publication boundary

The repository may be prepared and tested locally as v9.4.0, but committing,
pushing, tagging and publishing artifacts remain explicit release-manager
actions. This document does not authorize any of them.
