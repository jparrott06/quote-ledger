# Client migration checklist (gRPC / protobuf)

Use this when bumping `quote_ledger` server versions or regenerating stubs.

## Before upgrading

- [ ] Read release notes / milestone issues for breaking RPC or semantics changes.
- [ ] Confirm `SubscribeQuote` contract (reconnect, `after_seq`, error handling) matches your client.

## Stub upgrade

- [ ] Regenerate protobuf stubs for your language (Rust `tonic-build`, Go `protoc-gen-go-grpc`, etc.).
- [ ] Re-run compile on the client monorepo.
- [ ] Run client integration tests against a pinned server version in CI.

## Runtime configuration

- [ ] If auth is enabled (`QUOTE_LEDGER_AUTH_TOKEN`), thread the `authorization: Bearer …` metadata on all ledger RPCs.
- [ ] If TLS is enabled (`GRPC_TLS_CERT` / `GRPC_TLS_KEY` on server), point clients at `https` / TLS roots as appropriate.

## Rollout

- [ ] Canary a small slice of traffic against the new server build.
- [ ] Watch append latency and append stream outcome metrics during the canary.
- [ ] Roll forward or roll back with a documented decision.
