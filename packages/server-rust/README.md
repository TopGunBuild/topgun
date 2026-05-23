# topgun-server

The TopGun Rust server. Single binary built on [axum](https://docs.rs/axum) and [tokio](https://tokio.rs); ships the CRDT merge engine, WebSocket sync protocol, BM25 + HNSW search, cluster routing, and the embedded redb / optional Postgres backends.

Internal workspace crate — depends on [`topgun-core`](../core-rust/) for the CRDT primitives. Not published to crates.io; distribution is the Docker image at `ghcr.io/topgunbuild/topgun-server`.

## Quick start

From the repository root:

```bash
# Compile and run with the embedded redb backend (writes to ./topgun.redb)
pnpm start:server

# Or directly with cargo
cargo run --release --bin topgun-server
```

The binary refuses to boot without `JWT_SECRET` set (or `TOPGUN_NO_AUTH=1`). For local hacking:

```bash
TOPGUN_NO_AUTH=1 pnpm start:server
```

## Layout

```
src/
├── bin/topgun_server.rs   # entrypoint — wires services + axum
├── network/               # WebSocket handlers, HTTP /sync, admin, /health
├── service/
│   ├── domain/            # CRDT, Sync, Query, Search, Messaging, Persistence, Coordination
│   ├── dispatch/          # PartitionDispatcher — 271-partition routing
│   ├── middleware/        # Tower middleware (auth, observability, rate-limit)
│   └── policy/            # ACL / write-validator
├── storage/               # Backends (redb, postgres, null) + eviction + write-behind
├── cluster/               # Quorum election, membership, failure detection, peer connection
└── sim/                   # Deterministic in-memory simulation harness (feature `simulation`)

benches/load_harness/      # In-process Hyper-WS load test (cargo bench --bench load_harness)
docs/profiling/            # Flamegraphs + analysis notes
```

## Testing

```bash
# Unit tests (release build for realistic perf)
SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server

# Simulation tests (deterministic fault-injection)
pnpm test:sim

# In-process load harness
cargo bench --bench load_harness -- --connections 50 --duration 10
```

## Production tuning

All env knobs (RAM ceilings, eviction water marks, write-behind cadence, CORS, JWT, etc.) are in [`/docs/reference/server`](https://topgun.build/docs/reference/server) and the [Configuration guide](https://topgun.build/docs/deploy/configuration). The startup `tracing::info!` line summarises the effective config so operators can confirm without reading source.

## Contributing

See the repo-root [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the workflow. Rust-specific style is enforced by `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings` — both run in CI.
