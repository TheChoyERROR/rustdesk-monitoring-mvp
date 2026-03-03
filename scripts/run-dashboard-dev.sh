#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DASHBOARD_DIR="$ROOT_DIR/web-dashboard"
PORT="${1:-5173}"

if [ ! -d "$DASHBOARD_DIR" ]; then
  echo "No existe carpeta dashboard: $DASHBOARD_DIR"
  exit 1
fi

cd "$DASHBOARD_DIR"

if [ ! -d "node_modules" ]; then
  echo "Instalando dependencias del dashboard..."
  npm install
fi

exec npm run dev -- --host 0.0.0.0 --port "$PORT"
