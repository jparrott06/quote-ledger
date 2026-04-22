#!/usr/bin/env bash
# Backup quote-ledger SQLite DB using sqlite's online backup API.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/backup_sqlite.sh <db_path> <backup_path>

Example:
  scripts/backup_sqlite.sh /var/lib/quote-ledger/quote_ledger.db /backups/quote_ledger-$(date +%F-%H%M%S).db
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

DB_PATH="${1:-}"
BACKUP_PATH="${2:-}"

if [[ -z "$DB_PATH" || -z "$BACKUP_PATH" ]]; then
  usage
  exit 2
fi

if [[ ! -f "$DB_PATH" ]]; then
  echo "error: db file does not exist: $DB_PATH" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "error: sqlite3 CLI is required" >&2
  exit 1
fi

mkdir -p "$(dirname "$BACKUP_PATH")"
tmp_path="${BACKUP_PATH}.tmp.$$"
rm -f "$tmp_path"

escaped_tmp=${tmp_path//\'/\'\'}
sqlite3 "$DB_PATH" ".timeout 5000" ".backup '$escaped_tmp'"
mv "$tmp_path" "$BACKUP_PATH"

echo "backup created: $BACKUP_PATH"
