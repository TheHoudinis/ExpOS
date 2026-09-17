# ExpOS Go SDK

`expos.dev/sdk/expos` is the Go-facing contract for ExpOS Go ABI v1. It uses
FIN caller identity and a Form Handle on every privileged request—there are no
Unix paths, file descriptors, UIDs, or implicit global authority.

Implemented client calls cover surface creation, buffer attachment, damage,
atomic commit, browser navigation, event polling, Form resolution, Handle
authorization and package transactions. The included deterministic emulator
lets ordinary Go tooling test Form clients now:

```sh
go test ./...
```

The SDK is real and versioned, but the current v8 kernel does not yet load a
standard Go executable. Native execution requires the planned Form
implementation loader, scheduler, memory isolation and syscall entry path.
Until those land, the emulator verifies ABI behavior and the kernel's native
Rust Browser demonstrates the same surface protocol directly.
