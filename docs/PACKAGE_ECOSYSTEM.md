# Ayo package ecosystem

Ayo v3 is the package trust and transaction layer for ExpOS. The repository
ships a 25-package seed catalog today. The next ecosystem milestone is **50
reviewed, working packages**, not 50 placeholder names.

The intended public service is `packages.expos.org`; that hostname is a target,
not a claim that a production service is live. A production deployment should
publish a small signed index plus immutable content-addressed artifacts:

```text
packages.expos.org/
  v3/catalog.json
  v3/keys/release-2026.pub
  artifacts/sha256/<digest>
```

Catalog categories are Editors, Developer tools, Games, Networking, Languages,
Utilities, and Graphics. Schema v3 package entries include:

- name, semantic version, summary, category, and target architectures;
- provided Forms and required Package Forms;
- requested capabilities and PIMP constraints;
- immutable artifact URL, format, and SHA-256;
- a checksum over the complete package metadata.

The catalog as a whole may carry an Ed25519 signature. For a public registry,
clients should pin a release public key with `--registry-key`; Ayo reports
whether that verification actually happened. Checksums detect accidental
damage but are never described as signatures.

Installation follows one fail-closed path:

```text
signed catalog metadata
        -> Ed25519/checksum verification
        -> dependency resolution
        -> DIESE authority check
        -> PIMP/compatibility validation
        -> bounded download + SHA-256 verification
        -> safe staging (no traversal or links)
        -> recoverable artifact transaction
        -> atomic Package Form state commit
        -> owned-file receipt
```

`ayo slap TextLab` uses this catalog path. `ayo glance editor` searches package
names, summaries, categories, and provided Forms. `ayo files TextLab` audits
the exact installed paths and digests. Ayo never runs install scripts or hooks.

## Package acceptance bar

A package counts toward the 50-package milestone only when it has:

1. a working implementation or data payload rather than a marker-only stub;
2. a unique Package Form identity and explicit Form ABI requirement;
3. least-authority capabilities and reviewed dependencies;
4. deterministic build/package instructions through the ExpOS SDK;
5. contract tests and, where applicable, a native QEMU integration test;
6. immutable artifact digest, signed catalog metadata, uninstall ownership, and
   rollback coverage; and
7. an honest architecture list—`ASL` only after that toolchain exists, with
   `x86_64` or `host` used for current implementations as appropriate.

The compiled seed catalog is useful offline and release-trusted, but its small
`.form` activation artifacts are not a substitute for a public repository full
of third-party native implementations. Connecting Ayo's JSON bridge to native
ExpFS Handles and deploying the signed registry are still required platform
work.
