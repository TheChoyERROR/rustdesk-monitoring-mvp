# Tareas Joel 26-Marzo-2026

## 17/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se realizo el analisis de estado real del proyecto y de la brecha funcional respecto al objetivo de helpdesk distribuido. Se documento que la base existente ya cubria backend en Rust, dashboard web, SQLite, outbox con webhook, presencia, exportacion CSV y una primera integracion con el fork de RustDesk para emitir eventos de monitoreo. Tambien se identifico como riesgo principal que el sistema todavia no cubria el flujo completo de helpdesk, especialmente solicitud de soporte, disponibilidad de agentes, asignacion automatica y cierre operativo.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se consolido la arquitectura objetivo del proyecto y se dejo claro el backlog tecnico necesario para evolucionar desde un MVP de monitoreo hacia una solucion de helpdesk. En paralelo se reviso el estado del build de Windows, detectando que faltaban dependencias de compilacion y que el camino correcto para empaquetar el fork debia preservar el binario corporativo exacto y su trazabilidad.

## 18/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se corrigio la capa de monitoreo para que la presencia remota y el estado de control activo quedaran consistentes. Se ajusto el payload de presencia emitido por el fork y se arreglo la forma en que el backend derivaba y mantenia los estados de participantes, actividad y control, con el objetivo de estabilizar la supervision en tiempo real.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se avanzo con la automatizacion del entorno Windows para poder compilar el fork y generar instaladores de forma repetible. Se agrego el flujo de bootstrap de maquina nueva, junto con scripts de preparacion de entorno, instalacion de dependencias y validacion automatizada del build para acelerar futuras reinstalaciones o compilaciones desde cero.

## 19/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: No quedo un commit mayor registrado en este repositorio durante esta fecha, pero el trabajo necesario del proyecto se concentro en revisar el estado del fork, dependencias pendientes, estrategia de build local frente a Docker y definicion del camino operativo para tener un instalador corporativo reproducible.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se mantuvo la preparacion tecnica para el siguiente bloque funcional, dejando como prioridad la integracion del modelo helpdesk, la futura consola operativa web y la necesidad de enlazar tickets, agentes y sesiones remotas dentro del mismo flujo.

## 20/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: El trabajo de este tramo estuvo orientado a consolidar criterios de operacion y trazabilidad del sistema. Se tomo como referencia la necesidad de que todo cambio futuro tuviera soporte de documentacion, scripts reutilizables y validacion practica para despliegue, build y pruebas de funcionamiento sobre Windows.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se siguio afinando el backlog del proyecto con foco en helpdesk, integracion cliente-agente y empaquetado. Se dejo asentado que el sistema debia evolucionar hacia una orquestacion real de incidencias, no quedarse solo en monitoreo y auditoria de sesiones.

## 21/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: No quedo una entrega puntual versionada en esta fecha, pero se sostuvo trabajo de alineacion funcional y tecnica sobre el flujo objetivo: cliente final, agente, ticket, asignacion, atencion y cierre. Se priorizo la necesidad de desacoplar lo operativo de lo meramente visual para que el dashboard pudiera servir tanto para auditoria como para coordinacion.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se mantuvo la preparacion de despliegue y build, revisando como conectar frontend, backend y el fork corporativo con la menor friccion posible. Esto era necesario para poder pasar del analisis a una prueba integrada real en Render y Windows.

## 22/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se continuo con tareas de organizacion tecnica, pruebas y preparacion del repositorio para el siguiente salto funcional. La prioridad seguia siendo dejar listo el terreno para implementar helpdesk real, documentarlo bien y evitar cambios aislados sin trazabilidad.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se reforzo la idea de mantener el repositorio con scripts reproducibles, manifiestos de paquete e instalacion controlada. Esta etapa fue importante para que el trabajo posterior sobre agentes, tickets e instalador pudiera apoyarse en una base estable y no improvisada.

## 23/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se mantuvo trabajo de revision previa al bloque helpdesk, ordenando decisiones funcionales, modelo de estados y criterios de integracion entre backend, dashboard y fork de RustDesk. La necesidad principal era que el siguiente avance no solo mostrara datos, sino que permitiera operar incidencias.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se reviso el impacto de mover el proyecto hacia una capa de helpdesk sobre la base actual de monitoreo, dejando preparado el camino para definir entidades, endpoints, estados y paneles operativos que se versionaron en los dias posteriores.

## 24/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se implemento el nucleo del modulo helpdesk en backend. Se agregaron modelos, tablas, almacenamiento y API para agentes, tickets, asignaciones, heartbeats y auditoria operativa. Con esto el proyecto dejo de ser solo un backend de eventos y paso a tener una primera base funcional para gestionar solicitudes de soporte y agentes de atencion.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se construyo la primera capa del dashboard de helpdesk y su integracion con la API. Ademas, se documentaron la especificacion tecnica, el modelo de estados y el plan de implementacion, con el objetivo de que el flujo helpdesk tuviera coherencia operativa y fuera mantenible. Esta fecha marca el inicio claro del salto desde monitoreo hacia una operacion de soporte centralizada.

## 25/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se estabilizo el build corporativo de Windows y se hizo reproducible el flujo de bootstrap e instalador. Se generaron instaladores de prueba, se ajusto el empaquetado y se dejo el proceso mas fiable para reutilizarlo en nuevas maquinas o nuevas versiones del binario. En paralelo se desplegaron cambios importantes de helpdesk: visualizacion de avatar de agentes, flujo de asignacion y despacho desde dashboard, asi como auto-conexion del agente a tickets asignados.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se reforzo la experiencia web y operativa. Se mejoro la autenticacion del dashboard para que la sesion no se cerrara tan facilmente, pasando a una cookie firmada y renovable con keep-alive en frontend. Tambien se rediseño la vista de sesiones con un timeline visual por usuario usando ECharts, sustituyendo la lectura basada solo en tabla. Ademas, se reviso la situacion del webhook, se migro la recepcion temporal a Cloudflare Workers y se detecto como riesgo operativo la persistencia efimera de SQLite en Render Free.

## 26/03/2026

### Tareas realizadas de 9:00 a 13:00:

RustDesk Monitoring MVP: Se consolidaron notas operativas y de mantenimiento necesarias para continuar el proyecto sin perder contexto. Se definio que, para mantener historico de tickets y sesiones en Render, SQLite solo es viable si el servicio usa disco persistente en `/app/data`; en Render Free no hay garantia de persistencia. Como alternativa mas seria se dejo señalado que una futura migracion a Postgres seria el camino correcto para escalar.

### Tareas realizadas de 13:00 a 17:00:

RustDesk Monitoring MVP: Se aclaro el comportamiento esperado del timeline visual y se detecto que la representacion actual funciona tecnicamente, pero todavia no es la UX final deseada porque tramos con eventos aislados se expanden demasiado. Tambien se dejo documentado que el icono de la app en Windows debe partir idealmente de un `.ico` multiresolucion, y que cambiar branding visible a "TheChoy" es sencillo, mientras que una firma digital real de Windows requiere un certificado de code signing valido. Finalmente se solicito generar este documento de seguimiento para dejar registro por fechas desde el 17/03/2026 hasta el 26/03/2026.

## Notas importantes al 26/03/2026

- El backend y dashboard ya estan desplegados y automatizados para Render, pero SQLite en Render Free no garantiza persistencia. Para conservar historico real hace falta plan pago con persistent disk montado en `/app/data`, o bien migrar a Postgres.
- El dashboard ya incluye helpdesk, agentes, tickets, avatars, asignacion y timeline visual de sesiones, pero el timeline aun necesita refinamiento visual para representar mejor eventos puntuales o tramos incompletos.
- El instalador de Windows ya puede regenerarse y compartirse, y el proceso de build es mas reproducible que al inicio del periodo.
- La app puede personalizar icono y branding. Para Windows, el ejecutable toma principalmente `rustdesk-fork/res/icon.ico`. Para una firma digital reconocida por Windows no basta con poner el nombre de marca; hace falta certificado real.
- Se recomienda mantener separados tres frentes a partir de aqui: operacion helpdesk, persistencia/infraestructura y branding/distribucion del binario.
