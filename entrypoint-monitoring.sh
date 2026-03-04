#!/usr/bin/env sh
set -eu

PORT_VALUE="${PORT:-8080}"

set -- ./monitoring-server \
  --bind "0.0.0.0:${PORT_VALUE}" \
  --database-path /app/data/outbox.db \
  --config /app/server-config.railway.toml

if [ -n "${WEBHOOK_ENABLED:-}" ]; then
  set -- "$@" --webhook-enabled "${WEBHOOK_ENABLED}"
fi

if [ -n "${WEBHOOK_URL:-}" ]; then
  set -- "$@" --webhook-url "${WEBHOOK_URL}"
fi

if [ -n "${WEBHOOK_METHOD:-}" ]; then
  set -- "$@" --webhook-method "${WEBHOOK_METHOD}"
fi

if [ -n "${WEBHOOK_TIMEOUT_MS:-}" ]; then
  set -- "$@" --webhook-timeout-ms "${WEBHOOK_TIMEOUT_MS}"
fi

if [ -n "${WEBHOOK_MAX_ATTEMPTS:-}" ]; then
  set -- "$@" --webhook-max-attempts "${WEBHOOK_MAX_ATTEMPTS}"
fi

if [ -n "${WEBHOOK_BACKOFF_MS:-}" ]; then
  set -- "$@" --webhook-backoff-ms "${WEBHOOK_BACKOFF_MS}"
fi

if [ -n "${WEBHOOK_HMAC_ENABLED:-}" ]; then
  set -- "$@" --webhook-hmac-enabled "${WEBHOOK_HMAC_ENABLED}"
fi

if [ -n "${WEBHOOK_HMAC_SECRET:-}" ]; then
  set -- "$@" --webhook-hmac-secret "${WEBHOOK_HMAC_SECRET}"
fi

if [ -n "${WORKER_CONCURRENCY:-}" ]; then
  set -- "$@" --worker-concurrency "${WORKER_CONCURRENCY}"
fi

exec "$@"
