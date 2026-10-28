# Dexory Zenoh integration

This private repository preserves Eclipse Zenoh history and carries a coordinated
candidate for Hiroz, zenoh-router and rmw_zenoh. It is not an upstream fork in
GitHub's fork network: public forks cannot be made private. Public contributions
continue through botsandus/zenoh. Upstream licenses and notices are retained.

The base is Zenoh **1.10.2**. `integration/patches.json` records every imported PR
head and production backport. Each patch is a separate commit; the only manual
integration resolutions concatenate independently added session regression tests.
Upstream PRs, including #2873, are unchanged.

## Dependency policy

Pin the same full commit ID in every consumer of `zenoh`, `zenoh-ext` and other
Zenoh workspace crates. Use `cargo --locked`; verify Cargo.lock has one source
revision for the complete Zenoh family. Do not combine this with the old Hiroz
1.9 vendored crate, a registry zenoh-ext, or the old production mapping allocator.
Rust dependencies must use direct git/rev dependencies: root-only Cargo patches
do not propagate into downstream application workspaces.

Zenoh-C and C++ wrappers use their matching 1.10.2 releases. RMW must preserve the
base-topic buffer-endpoint workaround and include upstream rmw_zenoh #994. The
old core callback self-wait workaround is deliberately excluded because it
weakens the callback-drain guarantee.

Developers need read access to this repository. Configure Git authentication with
`gh auth setup-git`, then set `CARGO_NET_GIT_FETCH_WITH_CLI=true`. CI needs an
organization-approved read credential for this repository. Never put a token in
a Cargo URL, Docker ARG, build context, or image. Private dependency sources can
be fetched before packaging, or accessed through a BuildKit secret mount.

## Qualification

This branch is a test candidate. Passing component tests is not a fleet rollout
approval. Run the combined core regressions, Hiroz native and Go race suites,
production RMW interoperability, then the Noble/Resolute Balena soak. Record all
component commits and image IDs with the results. Exercise SHM on/off and separate
IPC namespaces, router restarts, discovery/endpoint churn, large payloads and
in-flight teardown. Replace all processes sharing an SHM cleanup namespace
together; an old cleanup implementation can still race a fixed creator.

Core regression commands (Rust 1.93.0):

```sh
cargo test --locked -p zenoh --no-default-features --features transport_tcp,transport_udp,shared-memory,unstable,internal --lib -- --test-threads=1
cargo test --locked -p zenoh-ext --no-default-features --features unstable,zenoh/transport_tcp,zenoh/shared-memory --test reentrancy -- --test-threads=1
cargo test --locked -p zenoh-shm --lib shm::unix::race_tests:: -- --test-threads=1
cargo build --locked -p zenohd --features shared-memory
```

The reconnect experiment and broader SHM diagnostic logging are not required by
the production baseline and are not carried here. Add any further fixes as
separate commits with a reproducer and recorded validation.
