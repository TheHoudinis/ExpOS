# ayo v2 development implementation

`ayo` is written entirely in Go and models packages as Package Forms with FIN
identity, Dimension state, revisions, capabilities, dependencies and a journal.
Its public command vocabulary matches the philosophy document.

The current executable runs outside the kernel. `JSONBridge` is explicitly a
development adapter for inspecting transactions; it is not the HexaOS storage
model. The kernel-side replacement will implement the same `Store` contract via
revocable Form Handles and HexaFS transactions.

Example:

```sh
make build
./bin/ayo --authority operator slap Browser 1.0.0
./bin/ayo glance
./bin/ayo vibecheck
```

