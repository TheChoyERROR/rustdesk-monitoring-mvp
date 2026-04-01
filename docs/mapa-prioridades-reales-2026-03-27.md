# Mapa de prioridades reales

Fecha: 2026-03-27

## Objetivo

Traducir la lista de issues posterior a la demo a un mapa de prioridades realista, alineado con el estado actual del codigo y con el tipo de riesgo que cada punto introduce en operacion.

## Lectura rapida

No todo lo pendiente tiene el mismo peso.

La prioridad real del proyecto hoy no es "hacer mas features", sino asegurar:

1. persistencia confiable;
2. asignacion robusta;
3. observabilidad operativa;
4. correlacion completa entre ticket y sesion real;
5. estabilidad de despliegue y webhook productivo.

## Leyenda

- `P0`: bloquea piloto serio u operacion estable.
- `P1`: no bloquea demo, pero deberia entrar en el siguiente tramo corto.
- `P2`: mejora importante para operacion mantenible.
- `P3`: refinamiento o deuda no urgente.
- `No prioridad ahora`: ya existe base suficiente; no debe entrar como bloque nuevo principal.

## P0. Bloqueadores reales de operacion

### ISSUE 13. Persistencia real en Render con disco montado

Prioridad real: `P0`

Por que:

- hoy el mayor riesgo no es funcional, es operativo;
- si SQLite vive en entorno efimero, se puede perder historico de tickets y sesiones;
- sin persistencia confiable, cualquier mejora funcional queda fragil.

Decision recomendada:

- mover el servicio a plan con disk persistente en `/app/data`;
- o preparar migracion posterior a Postgres.

### ISSUE 15. Webhook productivo endurecido

Prioridad real: `P0`

Por que:

- el emisor y el outbox ya funcionan;
- lo debil es depender de un receptor temporal o poco gobernado;
- si el webhook es parte importante del monitoreo externo, este punto es critico para operacion real.

Decision recomendada:

- receptor estable;
- HMAC activo;
- monitoreo y troubleshooting claros.

## P1. Siguiente tramo corto

### ISSUE 2. Fairness / reparto de carga entre agentes

Prioridad real: `P1`

Por que:

- la asignacion ya existe;
- lo que falta es una politica justa y explicable;
- este es el verdadero siguiente hueco del motor de despacho.

### ISSUE 11. Metricas operativas y SLA basicos

Prioridad real: `P1`

Por que:

- sin metricas, no hay forma seria de medir respuesta ni calidad;
- ya existe dashboard, pero falta convertirlo en panel operativo medible.

### ISSUE 6. Auditoria completa del ciclo de atencion

Prioridad real: `P1`

Por que:

- ya hay auditoria del ticket;
- falta cerrar mejor la relacion `ticket -> agente -> sesion remota -> cierre`;
- esto es clave para trazabilidad y soporte posterior.

## P2. Importantes, pero no bloquean la siguiente validacion

### ISSUE 14. Runbook operativo de incidentes

Prioridad real: `P2`

Por que:

- reduce dependencia del contexto oral;
- mejora soporte, mantenimiento y continuidad del sistema.

### ISSUE 9. Cola operativa en tiempo real

Prioridad real: `P2`

Por que:

- la cola ya existe con polling y filtros;
- lo siguiente es volverla mas viva y mas comoda para operacion continua.

### ISSUE 18. Estrategia de mantenimiento del fork contra upstream

Prioridad real: `P2`

Por que:

- es un riesgo estructural del producto;
- no rompe la demo ni el corto plazo, pero si no se ordena, el costo futuro sube mucho.

### ISSUE 19. Roles minimos adicionales

Prioridad real: `P2`

Por que:

- hoy puede operar un supervisor unico;
- en cuanto crezca el uso, conviene separar permisos y responsabilidades.

## P3. Refinamiento o mejora no urgente

### ISSUE 12. Refinamiento del timeline visual

Prioridad real: `P3`

Por que:

- el timeline ya funciona;
- hoy el problema es mas de UX y precision visual que de bloqueo funcional.

## No prioridad ahora como bloque nuevo

Estos issues no deben venderse como trabajo "faltante desde cero". Ya tienen una base real en el sistema:

### ISSUE 1. Motor de despacho desacoplado

Estado real:

- ya existe capa de asignacion central en backend;
- lo pendiente es formalizarla mejor y mejorar la politica de seleccion.

Prioridad real: `No prioridad ahora`

### ISSUE 5. Maquina de estados formal

Estado real:

- ya existe modelo de estados y transiciones reales en backend;
- lo pendiente es refinamiento de taxonomia, no implementacion inicial.

Prioridad real: `No prioridad ahora`

### ISSUE 7. Heartbeats y presencia operativa de agentes

Estado real:

- ya existe heartbeat, estados operativos y caida automatica a `offline`.

Prioridad real: `No prioridad ahora`

### ISSUE 10. Vista operativa de agentes

Estado real:

- ya existe vista de agentes, estados, ticket actual y ultimo heartbeat;
- lo pendiente es enriquecerla.

Prioridad real: `No prioridad ahora`

### ISSUE 16. Pipeline reproducible del instalador Windows

Estado real:

- ya existe flujo de bootstrap, build, manifiesto y artefactos reproducibles;
- lo pendiente es endurecer y formalizar release.

Prioridad real: `No prioridad ahora`

### ISSUE 17. Trazabilidad formal del binario corporativo

Estado real:

- ya existe hash, version y manifiesto de paquete;
- lo pendiente es formalizar mas la relacion release/commit/binario.

Prioridad real: `No prioridad ahora`

### ISSUE 20. Endurecimiento de sesion web

Estado real:

- ya se reforzo cookie, renovacion y keep-alive;
- no es una urgencia principal en este momento.

Prioridad real: `No prioridad ahora`

## Mapa final resumido

### P0

- `ISSUE 13` persistencia real en Render
- `ISSUE 15` webhook productivo endurecido

### P1

- `ISSUE 2` fairness / reparto de carga
- `ISSUE 11` metricas operativas y SLA
- `ISSUE 6` auditoria completa ticket-agente-sesion

### P2

- `ISSUE 14` runbook operativo
- `ISSUE 9` cola operativa mas viva
- `ISSUE 18` mantenimiento del fork
- `ISSUE 19` roles adicionales

### P3

- `ISSUE 12` refinamiento del timeline

### No prioridad ahora

- `ISSUE 1`
- `ISSUE 5`
- `ISSUE 7`
- `ISSUE 10`
- `ISSUE 16`
- `ISSUE 17`
- `ISSUE 20`

## Conclusión

La prioridad real del proyecto no es rehacer piezas ya existentes, sino fortalecer lo que convierte el MVP en una solucion operable:

- persistencia;
- despacho justo;
- metricas;
- auditoria completa;
- webhook y despliegue confiables.
