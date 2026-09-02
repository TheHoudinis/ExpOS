# ayo v2

`ayo` is the Go implementation of the HexaOS Package Form manager. Packages
are persistent Forms identified by FIN and scoped to a Dimension; they are not
archives copied into Unix-style directories.

## Interactive catalog

Running `ayo` without a command opens the Package Form deck:

```sh
./bin/ayo --authority operator
```

Choose a package number/name to preview and install it. The TUI searches with
`/text`, shows details with `d NAME`, lists installed Forms with `i`, and
removes with `x NAME`. Dependencies are resolved before confirmation and the
entire plan commits as one transaction. The built-in starter catalog works
offline.

Remote registries are metadata catalogs because HexaOS packages are Forms, not
file archives:

```sh
./bin/ayo --authority operator \
  --registry https://packages.example/ayo-v2.json \
  --registry-key BASE64_ED25519_PUBLIC_KEY
```

Remote sources must use HTTPS, remain below 2 MiB, and include a valid SHA-256
checksum for every Package Form. When a public key is supplied, the complete
catalog must also pass Ed25519 signature verification.

Implemented command vocabulary (unchanged from the philosophy):

- `slap` activates or revises a Package Form after dependency validation;
- `yeet` revokes activation, with dependent protection and optional `--force`;
- `glance` inspects identity, state, capabilities and relationships;
- `chill` reconciles desired PIMP state and dependency availability;
- `fix` normalizes metadata and repairs activation state;
- `ghost` hides a Form while retaining its FIN and history;
- `manifest` creates a named, checksummed revision checkpoint;
- `highfive` performs a controlled capabilities/provided-Forms merge;
- `dodge` excludes a Form from future reconciliation;
- `vibecheck` validates FINs, state and versioned dependencies;
- `flex` reports Package Form statistics.

Example:

```sh
make build
./bin/ayo --authority operator slap --cap socket --provide NetworkService Network 2.1.0
./bin/ayo --authority operator slap --dep 'Network@>=2.0.0' Browser 1.0.0
./bin/ayo glance Browser
./bin/ayo vibecheck
./bin/ayo --authority operator manifest known-good
```

The JSON bridge models HexaFS transaction boundaries for host development. It
uses a single-writer lock, validates a cloned state before commit, writes a
pending journal, syncs and atomically renames the next state, and retains the
previous valid commit for recovery. It remains a bridge—not the final on-disk
HexaFS format. Native integration will replace it with kernel Form Handle
calls; the authoritative package engine and all command semantics stay in Go.
