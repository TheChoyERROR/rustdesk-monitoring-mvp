## 🧩 Funcionalidades propuestas para el fork

### 1) 🎥 Control de grabaciones mediante argumentos de línea de comandos
Habilitar la gestión de grabación de sesiones (entrantes y salientes) a través de parámetros CLI.

**Capacidades:**

- Activar o desactivar grabación automáticamente  
- Iniciar o finalizar grabación por script  
- Diferenciar entre sesiones entrantes y salientes  
- Integración con herramientas de automatización y despliegue  

**Objetivo:** facilitar auditoría, cumplimiento normativo y automatización en entornos profesionales.

---

### 2) 🔔 Webhook de notificación de estado de sesión
Enviar notificaciones HTTP a un endpoint configurable cuando una sesión remota se inicie o se cierre.

**Capacidades:**

- Configuración de endpoint (URL) y método (POST/PUT)  
- Notificación al levantarse una sesión (session_started)  
- Notificación al cerrarse una sesión (session_ended)  
- Inclusión de metadatos (ID de sesión, usuario, tipo: entrante/saliente, timestamp)  
- Reintentos automáticos ante fallos y firma opcional (HMAC)  

**Objetivo:** habilitar integración con sistemas externos (monitoreo, auditoría, SIEM, automatización) en tiempo real.

### 3) 👤 Presencia remota con avatar identificable
Mostrar visualmente la presencia de usuarios conectados mediante avatares o identificadores gráficos.

**Incluye:**

- Avatar o icono por usuario conectado  
- Indicador de quién tiene el control activo  
- Señales visuales de actividad  
- Integración con sesiones colaborativas  

**Objetivo:** mejorar la claridad, la coordinación y la experiencia de usuario en sesiones multiusuario.