# SQLite schema migrations

The service applies SQL migrations from the `migrations/` directory on startup (`sqlite::open_and_migrate`). Migrations are **ordered by filename** and run in a single SQLite transaction per startup batch.

## Operator expectations

- **Backup before upgrades**: take a filesystem copy of `QUOTE_LEDGER_DB` (see `scripts/backup_sqlite.sh`) before deploying a binary that adds migrations.
- **One writer**: only one `quote_ledger` process should open the database file at a time for the current single-node design.
- **WAL and pragmas**: after migrations, the connection enables WAL, busy timeout, and related durability settings (`src/sqlite.rs`). Changing these at runtime outside the binary is unsupported.

## Adding a migration

1. Add a new file `migrations/NNN_description.sql` with a monotonically increasing `NNN` prefix.
2. Prefer additive changes (new tables/columns/indexes); avoid rewriting large tables in-place during migration.
3. Document any required one-off data backfill in the PR that introduces the migration.
