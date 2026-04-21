#!/usr/bin/env bash
# One-time: labels, Epic A milestone, and story issues. Safe to re-run labels (ignored if exist).
set -euo pipefail

REPO="${REPO:-jparrott06/quote-ledger}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

gh label create "type/story" --color "0E8A16" --description "Implementable story" -R "$REPO" 2>/dev/null || true
gh label create "type/epic" --color "5319E7" --description "Epic tracking" -R "$REPO" 2>/dev/null || true
gh label create "epic/A" --color "FBCA04" --description "Epic A — Domain" -R "$REPO" 2>/dev/null || true

gh api -X POST "repos/${REPO}/milestones" \
  -f title="Epic A — Domain model" \
  -f description="Pure quote state, commands→events, reducer, tests (no I/O)." \
  >/dev/null || true

create_issue() {
  local title="$1"
  local body_file="$2"
  gh issue create -R "$REPO" -t "$title" -F "$body_file" -l "type/story,epic/A" -m "Epic A — Domain model"
}

create_issue "A-01: QuoteState, DomainEvent, and reducer" "$ROOT/docs/planning/issues/a-01.md"
create_issue "A-02: Seq and ordering ownership (documentation)" "$ROOT/docs/planning/issues/a-02.md"
create_issue "A-03: DomainCommand and command_to_events" "$ROOT/docs/planning/issues/a-03.md"
create_issue "A-04: Golden / replay test coverage" "$ROOT/docs/planning/issues/a-04.md"

echo "Done. Issues: https://github.com/${REPO}/issues"
