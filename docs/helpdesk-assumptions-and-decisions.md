# Supuestos y Decisiones de Implementacion

## Proposito
Este documento recoge las decisiones cerradas despues de la conversacion original para poder ejecutar el proyecto sin ambiguedad.

No sustituye a los requisitos originales. Los complementa.

## Decisiones cerradas

### Regla de asignacion
- La asignacion inicial sera al primer agente `available`.
- No se implementa por ahora logica por skills, score o prioridad avanzada.
- La estrategia debe dejarse desacoplada para poder cambiarla mas adelante.

### Apertura de sesion
- La apertura de la atencion sera automatica.
- Antes de abrir la sesion se mostrara al operador un aviso con cuenta atras de `10 segundos`.
- Si en ese intervalo el operador deja de estar disponible o cancela, la sesion no debe abrirse.

### Alcance
- Multiempresa queda fuera de alcance por ahora.
- Auditoria se tiene en cuenta desde el diseno, pero con prioridad baja frente al flujo operativo principal.

### Persistencia
- SQLite se mantiene como base de datos inicial.
- El esquema debe preparar una futura migracion a una base de datos mas robusta sin rehacer el modelo funcional.

### Tecnologia
- El fork cliente de RustDesk sigue en Rust/Flutter.
- El backend actual puede mantenerse en su tecnologia existente mientras cumpla el flujo requerido.

## Reglas operativas derivadas
- Un agente `busy` no puede recibir otra asignacion.
- Un ticket no puede quedar asignado a mas de un agente a la vez.
- Si no hay agentes disponibles, la solicitud debe quedar en cola.
- Si un agente asignado desaparece, expira o cancela antes de iniciar la atencion, la solicitud debe volver a cola.

## Requisitos tecnicos derivados
- Heartbeat de agentes para conocer disponibilidad real.
- Lock transaccional o equivalente para evitar doble asignacion.
- Estado intermedio de apertura para cubrir la cuenta atras de 10 segundos.
- Registro basico de eventos para trazabilidad operativa.

## Prioridades

### Alta
- Solicitud de ayuda desde el cliente final.
- Activacion/desactivacion del agente.
- Asignacion automatica al primer agente disponible.
- Apertura automatica con aviso de 10 segundos.
- Bloqueo de doble asignacion.

### Media
- Cola de tickets cuando no haya agentes.
- Reasignacion por timeout o abandono.
- Vista operativa de agentes y tickets.

### Baja
- Auditoria detallada.
- Reglas avanzadas de reparto.
- Multiempresa.

## Riesgos asumidos por ahora
- SQLite puede quedarse corto si la carga sube mucho, pero es suficiente para MVP.
- La asignacion al primer agente disponible puede no repartir carga de forma optima.
- La automatizacion de apertura obliga a manejar bien estados transitorios y errores de carrera.
