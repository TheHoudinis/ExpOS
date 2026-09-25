# ExpOS Rust SDK

`expos-sdk` is the `no_std` Rust client for ExpOS Form ABI v1. It reuses the
kernel-independent ABI types from `expos-core`, so Rust clients cannot silently
drift from the kernel's call numbers or 72-byte request / 40-byte response
layout.

The caller supplies a `Transport`. Host tests can provide a deterministic
transport now; the native transport remains intentionally unavailable until
the user-mode call gate exists.

```rust
use expos_sdk::{Client, Fin};

let mut form = Client::new(Fin::from_u128(7), display_handle, transport).unwrap();
let surface = form.create_surface(20, 20, 640, 480, 1)?;
```

Run `cargo test -p expos-sdk` from the repository root.
