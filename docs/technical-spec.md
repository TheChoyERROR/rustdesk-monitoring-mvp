# Especificacion tecnica MVP (v1.1)

## Objetivo
Implementar un flujo de auditoria en tiempo real con:
- Control de grabacion por CLI.
- Ingestion de eventos de sesion.
- Entrega webhook robusta con idempotencia y reintentos.
- Presencia remota colaborativa (participantes y control activo).

## API interna

### `POST /api/v1/session-events`

Encola un evento `SessionEventV1` para entrega asyncrona por webhook.

Respuesta:
- `202 Accepted`: evento encolado.
- `409 Conflict`: `event_id` ya existe.
- `400 Bad Request`: payload invalido.
- `500 Internal Server Error`: error inesperado.

### `GET /api/v1/sessions/:session_id/presence`

Consulta snapshot de presencia de una sesion.

Respuesta:
- `200 OK`: snapshot disponible.
- `404 Not Found`: no hay estado de presencia para la sesion.

### `GET /api/v1/sessions/:session_id/presence/stream`

Canal SSE para cambios de presencia en tiempo real.

Eventos SSE emitidos:
- `presence_snapshot`: estado actualizado de presencia.
- `presence_missing`: sesion sin presencia registrada.
- `presence_lagged`: el consumidor se atraso y hubo mensajes omitidos.
- `presence_error`: error interno al reconstruir snapshot.

### `GET /api/v1/sessions/presence`

Lista sesiones con participantes activos.

## Contrato `SessionEventV1`

```json
{
  "event_id": "f7688020-94ef-4c43-98fe-a5be3f6f6b22",
  "event_type": "participant_joined",
  "session_id": "ses-001",
  "user_id": "admin01",
  "direction": "outgoing",
  "timestamp": "2026-03-03T05:00:00Z",
  "host_info": {
    "hostname": "ws-01",
    "os": "linux",
    "app_version": "0.1.0"
  },
  "meta": {
    "participant_id": "alice",
    "display_name": "Alice",
    "avatar_url": "https://cdn.example/alice.png"
  }
}
```

`event_type` soportados:
- `session_started`
- `session_ended`
- `recording_started`
- `recording_stopped`
- `participant_joined`
- `participant_left`
- `control_changed`
- `participant_activity`

## Headers de webhook
- `x-event-id`
- `x-event-type`
- `x-timestamp`
- `x-signature` (si HMAC activo): `sha256=<hex>`
- `x-signature-version`: `v1`

Firma HMAC:
- Algoritmo: `HMAC-SHA256`
- Mensaje firmado: `"<timestamp>.<payload_json>"`

## Politica de reintentos
- Exponencial: `backoff_ms * 2^(attempt-1)`
- `max_attempts` configurable
- Al exceder intentos: estado `failed`

## Estados de outbox
- `pending`
- `processing`
- `delivered`
- `failed`

## Presencia remota (estado derivado)

Tabla `session_presence`:
- clave: `(session_id, participant_id)`
- campos: `display_name`, `avatar_url`, `is_active`, `is_control_active`, `last_activity_at`, `updated_at`

Reglas:
- `participant_joined`: activa participante y actualiza identidad visual.
- `participant_left`: desactiva participante y libera control.
- `control_changed`: limpia control previo y asigna control activo al nuevo participante.
- `participant_activity`: refresca actividad y mantiene participante activo.
- `session_ended`: desactiva todos los participantes de la sesion.

## Metricas disponibles (`GET /metrics`)
- `events_received_total`
- `webhook_sent_total`
- `webhook_failed_total`
- `webhook_retry_total`
- `webhook_latency_ms_sum`
- `webhook_latency_ms_count`

## Matriz de errores

| Caso | Codigo | Comportamiento |
|---|---:|---|
| JSON invalido o campo vacio | 400 | No encola |
| `event_id` duplicado | 409 | No duplica |
| Falla temporal webhook | N/A (interno) | Reintenta con backoff |
| Exceso de reintentos | N/A (interno) | Marca `failed` |
| HMAC habilitado sin secreto | startup error | Falla arranque server |
| Sesion sin presencia registrada | 404 | Respuesta `presence_not_found` |

## Persistencia local CLI
- Config: `~/.config/rustdesk-cli/config.json` (Linux) o equivalente en Windows.
- Estado runtime: `~/.config/rustdesk-cli/state.json`.
