# quote-ledger

Append-only **quote** ledger: commands become persisted events; clients **subscribe** over gRPC for snapshot + live tail. SQLite for storage. Rust + **tonic** + Protocol Buffers.

## What works today

- **Domain**: pure reducer; `CreateQuote`, `AddLineItem`, `FinalizeQuote`; deterministic **subtotal / tax / total** in minor units (`src/domain`).
- **Tax stub**: `US` and `US-*` jurisdictions use **800 bps** (8%); others **0**.
- **Persistence**: SQLite `events` + `idempotency_keys`, `seq` per `quote_id` (`src/store.rs`).
- **gRPC**: `AppendCommands` (client streaming) commits commands; `SubscribeQuote` (server streaming) emits catch-up **tail** (if needed), a **snapshot** (includes `line_items` + totals), then live **tails** driven by `watch` + DB reads (`src/ledger.rs`).

## Planning (GitHub Issues & Milestones)

Work is tracked as **Issues** grouped by **[Milestones](https://github.com/jparrott06/quote-ledger/milestones)** (one milestone per epic).

**v1 epics (closed):** Epic A — domain model; Epic B — SQLite event store; Epic C — `AppendCommands`; Epic D — `SubscribeQuote`; Epic E — line items, tax, finalize; Epic F — DX, CI, observability. Issue [#5](https://github.com/jparrott06/quote-ledger/issues/5) is the umbrella ship item for Epics B–D (see its milestone comment).

- [Open issues](https://github.com/jparrott06/quote-ledger/issues)
- Epic A bootstrap stories: [#1](https://github.com/jparrott06/quote-ledger/issues/1), [#2](https://github.com/jparrott06/quote-ledger/issues/2), [#3](https://github.com/jparrott06/quote-ledger/issues/3), [#4](https://github.com/jparrott06/quote-ledger/issues/4).

Use the **Story / Task** or **Epic** templates when filing new work.

## Releases

**[v1.0.0](https://github.com/jparrott06/quote-ledger/releases/tag/v1.0.0)** — first tagged **vertical slice**: domain through SQLite persistence, gRPC append/subscribe, pricing/finalize, reflection/CI, and Prometheus metrics + bounded append drain on shutdown. The crate version matches the tag; the package remains `publish = false` until you opt into crates.io.

## Build

Needs Rust **1.86+** (see `rust-toolchain.toml`).

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Run the gRPC server (creates/applies SQLite migrations):

```bash
export QUOTE_LEDGER_DB="${TMPDIR:-/tmp}/quote_ledger.db"
# Optional: Prometheus scrape endpoint (default 127.0.0.1:9090)
export METRICS_ADDR=127.0.0.1:9090
# Optional: require `authorization: Bearer <token>` on ledger RPCs
export QUOTE_LEDGER_AUTH_TOKEN=dev-local-token
# Reliability defaults (optional overrides shown)
export APPEND_IDLE_TIMEOUT_MS=10000
export APPEND_MAX_COMMANDS=512
export GRPC_CONCURRENCY_LIMIT=128
export GRPC_KEEPALIVE_INTERVAL_MS=30000
export GRPC_KEEPALIVE_TIMEOUT_MS=10000
cargo run -- 127.0.0.1:50051
```

### Metrics (`METRICS_ADDR`)

The process exposes **Prometheus** text exposition on **`GET /metrics`** (separate HTTP listener from gRPC).

- **`METRICS_ADDR`**: `host:port` (default **`127.0.0.1:9090`**). Example scrape: `curl -s http://127.0.0.1:9090/metrics`.
- Counters/gauges/histograms include in-flight append tracking (`quote_ledger_in_flight_appends`), append stream outcomes, subscribe opens, and append-one latency.

### Auth (`QUOTE_LEDGER_AUTH_TOKEN`)

When `QUOTE_LEDGER_AUTH_TOKEN` is set, `AppendCommands` and `SubscribeQuote` require gRPC metadata:

- `authorization: Bearer <token>`
- If unset, the service behaves as before (no auth gate).
- Reflection remains enabled to preserve local exploration workflows.

### Shutdown

On **Ctrl+C**, the gRPC server does **not** begin tonic shutdown until **in-flight `append_one` calls** drop to zero, or **30 seconds** elapse (whichever comes first). After that, `serve_with_shutdown` completes and the process exits.

### Reliability limits

- `APPEND_IDLE_TIMEOUT_MS` (default `10000`): if an `AppendCommands` stream is idle too long between frames, the call fails with `DEADLINE_EXCEEDED`.
- `APPEND_MAX_COMMANDS` (default `512`): caps command count per `AppendCommands` stream; overflow fails with `RESOURCE_EXHAUSTED`.
- `GRPC_CONCURRENCY_LIMIT` (default `128`): max concurrent RPCs per connection.
- `GRPC_KEEPALIVE_INTERVAL_MS` / `GRPC_KEEPALIVE_TIMEOUT_MS` (defaults `30000` / `10000`): HTTP/2 keepalive policy.

### Retry semantics

`AppendCommands` is safe to retry for transient transport failures when each command keeps the same `(quote_id, client_command_id)` pair. The service deduplicates by that idempotency key and returns the already committed events.

## CI

Pull requests run **fmt**, **clippy**, **test** (includes gRPC smoke), **release-build-smoke**, and a **`coverage`** job that prints an **llvm-cov** summary (line/region totals — informational unless you add a threshold).

Configure branch protection to require the first four checks before merging to `main`. Add `coverage` when you want PRs blocked on the summary existing (it does not upload to Codecov unless you wire that in).

## Coverage (honest picture)

- **Strengths:** domain logic has focused **unit tests**; **integration tests** cover subscribe (empty), append+idempotency, pricing/finalize, auth/reliability controls, and deterministic **concurrent append** + fault-path checks against real SQLite + tonic server.
- **Gaps:** no property/fuzz tests yet; no sustained long-run soak benchmark yet; **no enforced line-coverage floor** in CI (the `coverage` job reports numbers only).
- **Local:** `cargo install cargo-llvm-cov` then `cargo llvm-cov test --workspace --all-features --html` for an HTML report.

## Ops runbook (v1.1)

- Runbook: `docs/ops-runbook.md`
- Backup script: `scripts/backup_sqlite.sh <db_path> <backup_path>`
- Restore script: `scripts/restore_sqlite.sh <backup_path> <db_path>`

## grpcurl (reflection)

The server registers **gRPC reflection** (descriptor set from `build.rs`). With `grpcurl`:

```bash
grpcurl -plaintext localhost:50051 list
grpcurl -plaintext localhost:50051 describe quote_ledger.v1.QuoteLedgerService

# If auth is enabled:
grpcurl -plaintext -H "authorization: Bearer ${QUOTE_LEDGER_AUTH_TOKEN}" \
  localhost:50051 quote_ledger.v1.QuoteLedgerService/SubscribeQuote
```

The server shuts down on **Ctrl+C** after **draining in-flight appends** (bounded wait), then `serve_with_shutdown` completes.

## License

MIT OR Apache-2.0 (SPDX).
