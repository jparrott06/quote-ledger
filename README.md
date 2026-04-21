# quote-ledger

Append-only **quote** ledger: commands become persisted events; clients **subscribe** over gRPC for snapshot + live tail. SQLite for storage. Rust + **tonic** + Protocol Buffers.

## What works today

- **Domain**: pure reducer + `CreateQuote` command validation (`src/domain`).
- **Persistence**: SQLite `events` + `idempotency_keys`, `seq` per `quote_id` (`src/store.rs`).
- **gRPC**: `AppendCommands` (client streaming) commits commands; `SubscribeQuote` (server streaming) emits catch-up **tail** (if needed), a **snapshot**, then live **tails** driven by `watch` + DB reads (`src/ledger.rs`).

## Planning (GitHub Issues)

Work is tracked as **Issues** grouped by **Milestones** (one milestone per epic).

- [Open issues](https://github.com/jparrott06/quote-ledger/issues)
- Use **Issues → Milestones** to view epics (e.g. **Epic A — Domain model**).
- Epic A bootstrap stories: [#1](https://github.com/jparrott06/quote-ledger/issues/1), [#2](https://github.com/jparrott06/quote-ledger/issues/2), [#3](https://github.com/jparrott06/quote-ledger/issues/3), [#4](https://github.com/jparrott06/quote-ledger/issues/4).

Use the **Story / Task** or **Epic** templates when filing new work.

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
cargo run -- 127.0.0.1:50051
```

## CI

Pull requests run **fmt**, **clippy**, **test** (includes gRPC smoke), and **release-build-smoke**. Configure branch protection to require those checks before merging to `main`.

## License

MIT OR Apache-2.0 (SPDX).
