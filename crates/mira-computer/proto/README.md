# Vendored `cua.env.v1` contract

These `.proto` files are the wire contract that
[cua-spacesd](https://github.com/trycua/cua) serves on port 3211. They are
vendored verbatim from `libs/cua/proto/cua/env/v1/` in the `trycua/cua`
repository and are generated into a Rust client by `../build.rs`.

## Provenance

| | |
| --- | --- |
| Upstream | <https://github.com/trycua/cua> |
| Path | `libs/cua/proto/cua/env/v1/` |
| License | MIT — Copyright (c) 2025 Cua AI, Inc. (reproduced in `LICENSE`) |
| Vendored at | `cua-spacesd` v0.5.3, `cua.env.v1` protocol version 1, revision 8 |

Only the subset `mira-computer` needs is vendored: `common` and `computer`
(the [`ComputerBackend`] implementation) plus `system`, which pulls in
`diagnose`, `stream` and `windows` as imports. Everything else in
`cua/env/v1` — filesystem, process, presence, teleport, tunnel — is left out.

The contract is additive-only upstream: fields and enum values are never
renumbered, and a breaking change ships as a new package (`cua.env.v2`) served
alongside v1. When you refresh these files, re-run `cargo test -p mira-computer
--test cua_daemon`, which exercises the wire path against a stand-in daemon.

[`ComputerBackend`]: ../src/backend/mod.rs

## Refreshing

```sh
BASE=https://raw.githubusercontent.com/trycua/cua/main/libs/cua/proto/cua/env/v1
for f in common computer diagnose stream system windows; do
  curl -fsSL -o "proto/cua/env/v1/$f.proto" "$BASE/$f.proto"
done
```

Then update the "Vendored at" row above and run the test suite.

## License

MIT, © 2025 Cua AI, Inc. See `LICENSE`. Note that this covers the *contract*
only. The `cua-spacesd` daemon these protos describe is source-available under
FSL-1.1-MIT; Mira depends on that contract, not on that code.