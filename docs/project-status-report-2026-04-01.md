# Informe de estado del proyecto

Fecha de corte: 2026-04-01

## 1. Resumen ejecutivo

Desde el reporte del 2026-03-17 el proyecto avanzo de una base de monitoreo con helpdesk incompleto a un flujo mucho mas cercano a operacion real.

Hoy el sistema ya permite:

- crear y despachar tickets desde el dashboard;
- mostrar agentes con nombre y avatar en la web;
- diferenciar experiencia de agente vs cliente final dentro de la app desktop;
- autorizar que solo ciertos equipos puedan operar como agentes;
- crear tickets desde el cliente final con datos mas ricos;
- dejar la app residente en la bandeja de Windows para pedir soporte desde segundo plano;
- generar instaladores Windows nuevos del fork corporativo.

La base funcional ya no esta en "solo monitoreo". Ya existe un helpdesk operativo inicial. Lo que sigue siendo mas delicado no es la funcionalidad base, sino la robustez de produccion: persistencia real en Render, trazabilidad 100% reproducible del fork y refinamientos de operacion.

## 2. Cambios recientes mas importantes

### 2.1 Backend y dashboard

Cambios principales incorporados en el repositorio principal:

- `aa87cf8` `feat(helpdesk): surface agent avatars in dashboard`
- `a6b80f4` `feat(helpdesk): dispatch tickets from dashboard`
- `5b48fa5` `feat(dashboard): keep sessions alive and visualize activity`
- `28fd155` `feat(helpdesk): enrich client ticket intake`
- `e195b35` `feat(helpdesk): authorize operator devices`

Impacto funcional:

- el dashboard ya puede crear tickets con datos enriquecidos;
- el dashboard ya puede despachar tickets a agentes disponibles;
- la sesion web fue endurecida para no cerrarse tan rapido;
- la pagina de sesiones ahora tiene timeline visual en lugar de solo tabla plana;
- la operacion de agentes ya se apoya en una lista de dispositivos autorizados desde la web.

### 2.2 Fork desktop corporativo

Cambios principales incorporados en el fork:

- `4550776b6` `feat(helpdesk): auto-connect dispatched tickets`
- `46a88f3e2` `feat(helpdesk): split client and agent desktop modes`
- `320bcaa3c` `chore(build): track generated flutter bridge sources`
- `de7babd27` `feat(helpdesk): gate operator mode behind authorization`
- `10bcee32e` `feat(helpdesk): keep client app in tray on close`

Impacto funcional:

- la app desktop ya separa modo cliente final y modo agente;
- el switch local ya no basta para que un equipo se convierta en agente;
- solo los `RustDesk ID` autorizados desde dashboard pueden publicar presencia y operar como agente;
- el cliente final puede pedir soporte sin ver estados operativos de agente;
- al cerrar con `X`, la app queda en tray y puede lanzar la solicitud de soporte desde menu contextual.

### 2.3 Instalador y release local

La ultima build generada localmente es:

- `1.4.6-monitoring.20260331.3`

Artefactos:

- `artifacts/windows-installer/rustdesk-monitoring-corporate-1.4.6-monitoring.20260331.3/...-setup.exe`
- `artifacts/windows-installer/rustdesk-monitoring-corporate-1.4.6-monitoring.20260331.3/...-portable.zip`

Datos relevantes del paquete:

- `monitoring_url`: `https://rustdesk-monitoring-mvp.onrender.com`
- `source_exe_sha256`: `f9bd891e6495299f497d1ce254b3a24c9b5224165b4b79fd78e9714201853da9`

## 3. Estado actual del proyecto

## 3.1 Actores y flujo operativo

El modelo funcional actual ya se acerca al flujo de 3 actores pedido:

1. `Supervisor`
2. `Agente helpdesk`
3. `Cliente final / equipo atendido`

Estado actual por actor:

- `Supervisor`: usa dashboard para ver agentes, crear tickets, despachar tickets y auditar operacion.
- `Agente`: usa la app desktop en modo agente, publica presencia y recibe trabajo si su equipo fue autorizado desde la web.
- `Cliente final`: usa la misma app desktop, pero en modo cliente; puede pedir soporte y abrir ticket sin ver controles internos del agente.

Importante:

- no existen dos aplicaciones separadas;
- la separacion actual se hace por modo de uso y por autorizacion desde backend;
- esto reduce confusion sin duplicar mantenimiento del producto.

## 3.2 Helpdesk

Hoy ya existe:

- creacion de tickets desde dashboard;
- ticket enriquecido con `title`, `description`, `difficulty` y `estimated_minutes`;
- asignacion y despacho desde dashboard;
- presencia de agentes;
- auto-conexion inicial para tickets despachados;
- modo cliente final y modo agente;
- autorizacion explicita de dispositivos operadores;
- formulario rapido de solicitud de ayuda desde la app.

Lo que sigue pendiente o incompleto:

- fairness formal de reparto entre agentes;
- metricas operativas y SLA;
- correlacion mas fuerte `ticket <-> sesion remota`;
- refinamiento final del timeline visual;
- runbook de incidentes y operacion.

## 3.3 Dashboard de monitoreo

Estado actual:

- autenticacion local de supervisor;
- sesiones con timeline visual;
- presencia de usuarios;
- vista operativa de helpdesk;
- agentes con avatar;
- exportacion de reportes;
- cola y detalle de tickets.

Punto a tener en cuenta:

- la nueva vista visual de sesiones es mas util que la tabla anterior, pero todavia necesita refinamiento UX para representar mejor eventos puntuales vs intervalos estimados.

## 3.4 App desktop

Estado actual:

- perfil de monitoreo;
- avatar y nombre visible;
- modo agente solicitado localmente;
- validacion real de autorizacion desde backend;
- tray con accion `Request help`;
- cierre a segundo plano con aviso inicial;
- formulario de ticket para cliente final.

Punto a validar en pruebas reales:

- la experiencia exacta del tray en Windows debe validarse siempre con el instalador mas reciente, porque depende del build del fork y del comportamiento real de shell/tray del sistema.

## 4. Estado de despliegue y release

## 4.1 Repositorios

Ultimos commits principales considerados para el estado actual:

- repo principal: `e195b35`
- fork desktop: `10bcee32e`

## 4.2 Deploy web

La URL operativa sigue siendo:

- `https://rustdesk-monitoring-mvp.onrender.com`

Estado esperado:

- si Render sigue en auto-deploy desde `main`, el backend y dashboard deben tomar los cambios ya empujados.

Estado no verificado en este reporte:

- no se deja asentado aqui un smoke test live completo posterior a todos los ultimos pushes;
- conviene considerar el deploy como "codigo empujado y listo", pero sujeto a verificacion manual en entorno real.

## 4.3 Estado del binario e instalador

El instalador actual ya incorpora:

- separacion cliente/agente;
- autorizacion de agentes desde dashboard;
- flujo de solicitud de soporte desde tray;
- cierre a segundo plano con aviso.

Limitacion conocida:

- la build del instalador sigue saliendo desde el `rustdesk-fork` local actual;
- ese fork todavia tiene cambios locales historicos no versionados del todo;
- por eso el instalador mas reciente es util para pruebas y demo, pero el pipeline aun no puede considerarse 100% limpio y reproducible desde un checkout nuevo.

## 5. Riesgos y pendientes reales

### 5.1 Persistencia en Render

Sigue siendo el riesgo operativo mas importante.

Si el servicio sigue en Render Free con SQLite local:

- el historial puede perderse en reinicios o redeploys;
- los tickets resueltos y datos operativos no tienen persistencia fiable.

Solucion recomendada:

- mover el servicio a plan con `persistent disk` montado en `/app/data`, o migrar a una base de datos mas robusta.

### 5.2 Receptor de webhook

La arquitectura de webhook ya es correcta del lado emisor:

- outbox;
- reintentos;
- HMAC;
- estados `pending / delivered / failed`.

Lo que debe sostenerse operativamente es:

- usar un receptor estable;
- no depender de URLs temporales tipo `webhook.site`;
- monitorear entregas y fallos reales.

### 5.3 Reproducibilidad del fork

El fork de RustDesk sigue siendo un riesgo estructural:

- hay drift local no completamente absorbido en Git;
- futuras releases deberian salir desde un arbol limpio;
- conviene cerrar la brecha entre "lo empujado" y "lo que realmente se compila localmente".

### 5.4 Refinamientos operativos

Todavia no esta cerrado:

- fairness de despacho;
- metricas y SLA;
- auditoria completa `ticket <-> agente <-> sesion`;
- refinamiento final de timeline;
- roles adicionales si la operacion crece.

## 6. Prioridades recomendadas desde este punto

Orden sugerido:

1. asegurar persistencia real en Render;
2. cerrar pipeline reproducible del fork e instalador;
3. validar en entorno real el flujo completo `cliente final -> ticket -> agente -> sesion`;
4. agregar metricas operativas y SLA;
5. mejorar fairness y auditoria de asignaciones;
6. refinar timeline y UX operativa.

## 7. Conclusion

El proyecto ya no esta en una fase de idea o solo monitoreo. Hoy existe un MVP funcional de helpdesk corporativo sobre el fork de RustDesk, con dashboard operativo, separacion de cliente/agente, solicitud de ayuda desde la app y build instalable en Windows.

La madurez actual es buena para demo y pruebas dirigidas. Lo que falta para considerarlo listo para operacion mas seria no es "hacer el helpdesk", sino endurecer despliegue, persistencia, reproducibilidad y observabilidad.
