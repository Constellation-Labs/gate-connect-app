# Vendored harness manifest

`harnesses.json` is a byte copy of the gateway repo's manifest
(`Constellation-Labs/gate`, `harnesses.json` at the root): the one list of every
harness Gate supports and what it promises for each. **Do not edit it here.**
Change it in the gateway repo, then re-vendor:

```
ci/vendor-manifest.sh <path to a gate checkout>
```

That copies the file as committed at the checkout's HEAD (not the working tree,
so the copy is always a real commit) and rewrites `harnesses.json.sha256`. A hand edit without
the checksum fails `cargo test`, which is what makes the copy trustworthy.

`crates/core/src/manifest.rs` reads it with `include_str!`, so nothing loads it
from disk or the network at run time, and checks it against this app's own
lists: each integration's routing mechanism and Gate models support, the proxy
domain slugs, and the names the request-stamping table emits. A mismatch fails
the build and names the harness and the missing piece.
