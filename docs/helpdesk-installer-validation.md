# Validacion del instalador Windows ya generado

Artefacto detectado en este repo:

- `artifacts\windows-installer\rustdesk-monitoring-corporate-1.4.6\rustdesk-monitoring-corporate-1.4.6-setup.exe`
- `artifacts\windows-installer\rustdesk-monitoring-corporate-1.4.6\rustdesk-monitoring-corporate-1.4.6-portable.zip`

Archivos auxiliares presentes en el paquete:

- `launch-rustdesk.cmd`
- `launch-rustdesk.ps1`
- `MONITORING-POLICY.txt`

## Que valida este instalador

Segun [windows-installer.md](./windows-installer.md), el empaquetado corporativo no solo copia `rustdesk.exe`.
Tambien prepara launchers que inyectan `RUSTDESK_MONITORING_URL` para que el fork reporte al backend de monitoring/helpdesk.

## Flujo minimo de validacion

1. Arrancar backend:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-monitoring-server.ps1
```

2. Instalar `*-setup.exe` o descomprimir `*-portable.zip`.

3. Lanzar la app usando el acceso directo instalado o `launch-rustdesk.cmd`.
   No validar abriendo el `.exe` crudo si quieres comprobar la configuracion corporativa, porque el valor de `RUSTDESK_MONITORING_URL` lo inyecta el launcher.

4. Confirmar que la app arranca y que el operador/cliente puede abrir la UI principal.

5. Generar actividad desde el fork:
   - abrir la app;
   - establecer modo operador;
   - o iniciar una conexion/sesion segun el escenario a probar.

6. Validar en backend:

```powershell
curl.exe -s http://127.0.0.1:8080/metrics
curl.exe -s http://127.0.0.1:8080/api/v1/sessions/presence
curl.exe -s http://127.0.0.1:8080/api/v1/helpdesk/summary
curl.exe -s http://127.0.0.1:8080/api/v1/helpdesk/agents
```

7. Validar en dashboard:
   - abrir el dashboard;
   - comprobar que aparece presencia/sesiones nuevas;
   - comprobar que el operador cambia entre `away`, `available`, `opening` y `busy` segun el flujo.

## Checklist especifico del bloque helpdesk operador

- El switch `Disponible/Ausente` envia heartbeat al backend.
- La asignacion activa aparece en el panel superior.
- Si el ticket entra en `opening`, se muestra cuenta atras usando `opening_deadline_at`.
- Al llegar a cero, el operador intenta:
  - marcar la asignacion como iniciada;
  - abrir sesion remota contra `client_id` tratado como `peer id`.
- Si falla la apertura o el backend, el error queda visible en el panel.

## Limitaciones actuales

- La apertura automatica asume que `ticket.client_id` coincide con el `peer id` real de RustDesk.
- En este entorno no se pudo validar compilacion Flutter con `dart analyze` o `dart format`, asi que la validacion fuerte debe hacerse ejecutando el instalador o el build Windows ya generado.
