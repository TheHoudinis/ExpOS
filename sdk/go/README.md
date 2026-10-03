# ExpOS Go SDK

`expos.dev/sdk/expos` is the Go binding for the language-neutral ExpOS Form ABI
v1. It uses
FIN caller identity and a Form Handle on every privileged request—there are no
Unix paths, file descriptors, UIDs, or implicit global authority.

Implemented call numbers cover identity, time, IPC, surface creation, buffer
attachment, damage, atomic commit, browser navigation, events, storage,
networking, Form resolution, Handle authorization, and package transactions.
The included deterministic emulator
lets ordinary Go tooling test Form clients now:

```sh
go test ./...
```

The SDK is real and versioned, but the current v9 kernel does not yet load a
standard Go executable. The native scheduler, memory isolation and Form ABI
entry path now exist for bounded x86_64 capsules; a Go image/runtime loader is
the missing integration. Until it lands, the emulator verifies ABI behavior
and the kernel's native Rust Browser demonstrates the same surface protocol.
