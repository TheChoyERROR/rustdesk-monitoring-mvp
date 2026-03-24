# Requisitos Originales Helpdesk

## Fuente
Transcripcion del archivo local `C:\Users\Edward Mendoza\Desktop\2026-03-16 09-45-57.txt`.

Este documento recoge solo los requisitos funcionales expresados en la conversacion, sin mezclar decisiones tecnicas posteriores ni supuestos de implementacion.

## Objetivo del producto
Convertir el fork de RustDesk en una solucion de helpdesk legitima para soporte remoto masivo.

La idea base es:
- muchas maquinas cliente tienen instalado RustDesk corporativo;
- existe un grupo de operadores de helpdesk;
- cuando un cliente solicita ayuda, el sistema conecta automaticamente a un operador disponible con esa maquina;
- el sistema debe comportarse como una herramienta de soporte honrada y visible para el usuario final, no como software oculto.

## Tipos de maquina

### 1. Maquina cliente final
Equipo del usuario que recibe soporte.

Requisitos funcionales:
- Debe tener instalado el cliente corporativo basado en RustDesk.
- Debe mostrar claramente que el sistema de helpdesk esta activado.
- Debe permitir que el usuario solicite atencion.
- Debe poder ser atendido por un operador remoto cuando el sistema lo asigne.

### 2. Maquina operador helpdesk
Equipo de la persona que presta soporte.

Requisitos funcionales:
- Debe poder indicar si esta activa y disponible para recibir trabajo.
- Debe poder dejar de estar activa cuando no quiera recibir mas trabajo.
- Debe reflejar cuando esta ocupada atendiendo una incidencia.
- Debe participar en la conexion remota con la maquina cliente asignada.

## Flujo funcional principal
1. Hay una base instalada grande de clientes con el software corporativo.
2. Un cliente final indica que necesita ayuda.
3. El sistema central recibe esa solicitud.
4. El sistema central detecta que operadores estan disponibles.
5. El sistema asigna la incidencia a uno de esos operadores.
6. El operador se conecta remotamente a la maquina cliente.
7. Mientras el operador esta atendiendo, no debe recibir otro trabajo.
8. Cuando termina la atencion, el operador vuelve a quedar disponible o se desactiva.

## Requisitos del sistema central
- Debe existir un servicio central que reciba y sincronice eventos.
- Debe saber cuando un cliente pide ayuda.
- Debe saber cuando un operador esta activo o inactivo.
- Debe saber cuando un operador esta ocupado o libre.
- Debe decidir que operador recibe cada trabajo.
- Debe poner en contacto automaticamente a cliente y operador.

## Backend y persistencia
De la conversacion original se desprende lo siguiente:
- El servidor puede mantenerse tal como este si ya existe una implementacion funcional.
- La base de datos actual puede ser SQLite.
- No se exige cambiar la tecnologia del fork RustDesk, que debe seguir en Rust.

## Requisitos de producto y presentacion
- Debe presentarse claramente como una herramienta de helpdesk.
- Debe existir algun texto o aviso visible que deje claro que el sistema esta activado.
- La herramienta debe estar orientada a uso corporativo y soporte autorizado.

## Requisitos de documentacion
La conversacion pide separar la documentacion en dos partes:
- un documento con lo hablado originalmente;
- otro documento con supuestos, cambios o anexos necesarios para poder avanzar.

## Temas mencionados pero no cerrados en origen
Estos puntos aparecieron de forma general, pero no quedaron definidos con suficiente detalle en la conversacion original:
- regla exacta de asignacion entre varios operadores libres;
- confirmacion manual o automatica antes de abrir una sesion;
- estados formales de ticket/agente;
- estructura exacta del backend y de sus APIs;
- nivel de auditoria requerido;
- si debe existir soporte multiempresa.
