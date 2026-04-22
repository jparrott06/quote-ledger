# API & protobuf versioning (quote_ledger)

## Package naming

Breaking wire or behavioral changes should introduce a new `package` (for example `quote_ledger.v2`) **or** a clearly documented compatibility window where old clients continue to work.

## Field evolution

- Prefer **adding** fields and reserving numbers; avoid reusing field numbers.
- Mark deprecated fields in `.proto` comments and keep server behavior tolerant until clients migrate.
- For removals or semantic changes, bump the package version or gate behavior behind explicit configuration.

## Server rollout

1. Deploy a server build that **accepts both** old and new clients (if dual-read is required).
2. Migrate clients to the new stubs.
3. Remove compatibility paths after the agreed sunset date.

## Compatibility testing

- Keep at least one integration test per major client contract (append, subscribe, pricing).
- Run `grpcurl` against both old and new descriptors during migration.
