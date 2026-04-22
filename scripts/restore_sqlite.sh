#!/usr/bin/env bash
# Restore quote-ledger SQLite DB from a backup file.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/restore_sqlite.sh <backup_path> <db_path>

Notes:
  - Stop quote-ledger before restore to avoid open-writer conflicts.
  - Existing DB is moved to "<db_path>.pre-restore.<epoch>" before restore.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

BACKUP_PATH="${1:-}"
DB_PATH="${2:-}"

if [[ -z "$BACKUP_PATH" || -z "$DB_PATH" ]]; then
  usage
  exit 2
fi

if [[ ! -f "$BACKUP_PATH" ]]; then
  echo "error: backup file does not exist: $BACKUP_PATH" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "error: sqlite3 CLI is required" >&2
  exit 1
fi

mkdir -p "$(dirname "$DB_PATH")"

if [[ -f "$DB_PATH" ]]; then
  stamp="$(date +%s)"
  prev="${DB_PATH}.pre-restore.${stamp}"
  mv "$DB_PATH" "$prev"
  [[ -f "${DB_PATH}-wal" ]] && mv "${DB_PATH}-wal" "${prev}-wal" || true
  [[ -f "${DB_PATH}-shm" ]] && mv "${DB_PATH}-shm" "${prev}-shm" || true
  echo "previous database moved to: $prev"
fi

tmp_path="${DB_PATH}.tmp.$$"
rm -f "$tmp_path"

escaped_tmp=${tmp_path//\'/\'\'}
sqlite3 "$BACKUP_PATH" ".timeout 5000" ".backup '$escaped_tmp'"
mv "$tmp_path" "$DB_PATH"

echo "database restored to: $DB_PATH"
