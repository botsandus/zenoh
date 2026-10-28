# Dexory Zenoh integration

This branch in the public botsandus/zenoh fork carries a coordinated candidate
for Hiroz, zenoh-router and rmw_zenoh. Upstream licenses and notices are retained.
Upstream contribution branches remain independent of this integration branch.

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

The core source is public and requires no build credential. Use HTTPS git URLs
with an immutable `rev`; never embed credentials in dependency URLs.

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
cargo test --locked -p zenoh --no-default-features --features transport_tcp,transport_udp,shared-memory,unstable,internal,test --lib -- --test-threads=1
cargo test --locked -p zenoh-ext --no-default-features --features unstable,zenoh/transport_tcp,zenoh/shared-memory --test reentrancy --test history_retry -- --test-threads=1
cargo test --locked -p zenoh-shm --lib shm::unix::race_tests:: -- --test-threads=1
cargo build --locked -p zenohd --features shared-memory
```

The reconnect experiment and broader SHM diagnostic logging are not required by
the production baseline and are not carried here. Add any further fixes as
separate commits with a reproducer and recorded validation.

## Integration corrections

Combining mapping reclamation (#2835) with sender-owned routes (#2879) exposed a
second lifetime race: a cached route has no mapping lease after the table lock is
released. This candidate routes data and queries using full key expressions.
Control/declaration compression is retained; replies were already literal.
The deterministic `routed_messages_survive_sender_mapping_reclamation_before_enqueue`
regression retires the mapping between route selection and enqueue, for both
puts and queries. This trades additional key-expression header bytes for correct
routing during cleanup; throughput must be measured in the deployment soak.

The production history retry backport now preserves `max_samples` and `max_age`,
and rearms undelivered-history recovery when a retained sequenced source returns
after liveliness loss. The `history_retry_preserves_max_age` regression verifies
that an old cached sample stays excluded and a fresh publication is delivered.
