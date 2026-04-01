#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ -z "${TURSO_DATABASE_URL:-}" ]; then
  echo "Falta TURSO_DATABASE_URL"
  exit 1
fi

if [ -z "${TURSO_AUTH_TOKEN:-}" ]; then
  echo "Falta TURSO_AUTH_TOKEN"
  exit 1
fi

if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  export CARGO_TARGET_DIR="$HOME/.cargo-target/rustdesk-monitoring-mvp"
fi

cd "$ROOT_DIR"
exec cargo run --bin turso-helpdesk-smoke -- "$@"
