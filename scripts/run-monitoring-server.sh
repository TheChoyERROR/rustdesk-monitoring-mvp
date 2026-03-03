#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_PATH="${1:-$ROOT_DIR/server-config.example.toml}"
DB_PATH="${2:-$ROOT_DIR/data/outbox.db}"
BIND_ADDR="${3:-0.0.0.0:8080}"

if [ ! -f "$CONFIG_PATH" ]; then
  echo "No existe config: $CONFIG_PATH"
  exit 1
fi

mkdir -p "$(dirname "$DB_PATH")"

cd "$ROOT_DIR"
if [ ! -x "$ROOT_DIR/target/release/monitoring-server" ]; then
  echo "Compilando monitoring-server (release)..."
  cargo build --release --bin monitoring-server
fi

exec "$ROOT_DIR/target/release/monitoring-server" \
  --config "$CONFIG_PATH" \
  --database-path "$DB_PATH" \
  --bind "$BIND_ADDR"
