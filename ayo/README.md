# Ayo v3

Ayo is ExpOS's Go Package Form manager. It keeps Form identity, dependencies,
capabilities, PIMP state and revisions while also performing the job expected
of a real package manager: obtaining verified bytes and safely materializing
them under an explicit install root.

## Use it

Build and open the searchable TUI:

```sh
make build
./bin/ayo --authority operator
```

The built-in 25-package catalog works offline and every package installs a real
owned `.form` artifact. The TUI accepts a package number or name, `/text` to
search, `d NAME` for details, `i` for installed packages, and `x NAME` to
remove one.

The same operations are available directly:

```sh
./bin/ayo --authority operator slap TextLab
./bin/ayo --authority operator install TextLab # compatibility spelling
./bin/ayo glance TextLab
./bin/ayo files TextLab
./bin/ayo --authority operator yeet TextLab
./bin/ayo --authority operator recover
```

By default, state and artifacts live in user-owned XDG locations. Use `--state`
and `--root` for isolated or system-integrated deployments. Ayo rejects broad
system roots such as `/`, `/usr`, and `/etc`.

## Registries and artifacts

Select a local JSON catalog or an HTTPS registry:

```sh
./bin/ayo --authority operator \
  --registry https://packages.example/ayo-v3.json \
  --registry-key BASE64_ED25519_PUBLIC_KEY
```

Schema v3 catalogs carry category and target-architecture metadata, so `glance`
can search Editors, Developer tools, Games, Networking, Languages, Utilities,
and Graphics without flattening everything into a path list. Remote catalogs
are size-bounded and package metadata is checksummed. The
optional registry key requires a valid Ed25519 signature. Ayo v3 catalog
entries include an artifact source, SHA-256 digest, format and optional raw-file
target. Sources may be HTTPS, `file://`, local files or `builtin://`; supported
formats are raw, tar and tar.gz.

Downloads are bounded, staged and digest-checked before installation. Archive
paths must stay beneath the selected root; absolute paths, traversal, backslash
aliases, symlinks and hard links are rejected. Ayo records each file's path,
digest, size and mode, blocks ownership collisions, and only uninstalls an
owned file when its digest is unchanged. It intentionally runs no package
scripts or hooks.

An install plan resolves dependencies first, stages every artifact, then
publishes files and Package Form state as one recoverable transaction. Backups
and a journal permit rollback after a failed commit or recovery after an
interrupted process.

For a one-off local artifact:

```sh
./bin/ayo --authority operator slap \
  --source ./tool.bin --sha256 HEX_DIGEST --format raw \
  --target bin/tool --mode 755 Tool 1.0.0
```

The philosophy command vocabulary remains available: `slap`, `yeet`, `glance`,
`chill`, `fix`, `ghost`, `manifest`, `highfive`, `dodge`, `vibecheck`, and
`flex`. `slap NAME` is the normal catalog install path; `install` is its
compatibility spelling. `slap NAME VERSION` creates a local Package Form, and
artifact flags attach local bytes. Ayo v3 also adds `files` and `recover`.

The JSON state store is a development ExpFS bridge. It uses a single-writer
lock, pending journal, atomic rename and recovery snapshot. Native integration
will replace that bridge with kernel Form Handle calls without changing the
package transaction model.
