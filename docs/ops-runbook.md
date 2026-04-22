# quote-ledger ops runbook (v1.1)

This runbook covers baseline operations for the current single-process SQLite deployment.

## 1) SLO indicators

Use these indicators from `/metrics`:

- **Append latency SLI**: `quote_ledger_append_one_duration_seconds`
  - target suggestion: p95 < 250ms over 10m windows.
- **Append stream outcome health**: `quote_ledger_append_commands_streams_total{result=*}`
  - watch non-`ok` ratios (`invalid`, `timeout`, `too_many`).
- **In-flight pressure**: `quote_ledger_in_flight_appends`
  - sustained growth indicates writer pressure or downstream lock contention.
- **Subscribe load trend**: `quote_ledger_subscribe_streams_total`
  - useful for traffic-shift/context during incidents.
- **Subscribe DB resilience**: `quote_ledger_subscribe_db_retries_total`, `quote_ledger_subscribe_db_load_failures_total`, `quote_ledger_subscribe_stream_terminal_errors_total`
  - retries indicate transient SQLite pressure; terminal errors mean clients should reconnect and operators should investigate storage health.
- **Global gRPC rate limiting** (when `GRPC_RATE_LIMIT_RPS` is set): `quote_ledger_grpc_rate_limited_total`
  - sustained growth means legitimate traffic is competing for a low cap, or an abusive client; tune `GRPC_RATE_LIMIT_RPS` or add edge rate limiting.

## 2) Alert suggestions

- **High append latency**: p95 append duration above threshold for >10m.
- **Append error ratio spike**: (`invalid` + `timeout` + `too_many`) / total append streams > 5% for >5m.
- **In-flight saturation**: in-flight appends > 0 and rising steadily for >5m.
- **Instance down**: scrape target missing.

## 3) Incident triage checklist

1. Confirm process healthy and endpoints responsive:
   - gRPC on service port
   - `/metrics` on `METRICS_ADDR`
2. Check recent error ratio and latency:
   - `quote_ledger_append_commands_streams_total`
   - `quote_ledger_append_one_duration_seconds`
3. Check DB path and disk space (`QUOTE_LEDGER_DB`, free space).
4. If shutdown/restart is needed:
   - send SIGINT (`Ctrl+C`) and wait for bounded append drain logs.
5. If data integrity is in question:
   - take immediate backup (below) before any restore attempt.

## 4) Backup and restore

Scripts:

- `scripts/backup_sqlite.sh <db_path> <backup_path>`
- `scripts/restore_sqlite.sh <backup_path> <db_path>`

### Backup example

```bash
scripts/backup_sqlite.sh /var/lib/quote-ledger/quote_ledger.db /var/backups/quote_ledger-$(date +%F-%H%M%S).db
```

### Restore example

```bash
# stop service first
scripts/restore_sqlite.sh /var/backups/quote_ledger-2026-04-22-120000.db /var/lib/quote-ledger/quote_ledger.db
```

Restore behavior:

- existing DB is moved to `<db_path>.pre-restore.<epoch>`
- restore writes via sqlite backup API to a temp file then atomically renames

## 5) Post-restore validation

1. Start service with the restored DB.
2. Run `grpcurl list` and `describe` checks.
3. Execute a small append+subscribe smoke test.
4. Confirm `/metrics` is exposed and append stream outcomes remain stable.
