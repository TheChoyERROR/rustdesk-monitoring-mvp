# Setup del fork RustDesk (Ubuntu 24.04)

Esta guia documenta los prerequisitos de sistema para compilar el fork local de RustDesk en:
- `/home/choy/Escritorio/rustdesk`

## Problema detectado

Durante `cargo check` en el fork de RustDesk fallo la compilacion nativa por librerias de sistema faltantes:
- `glib-2.0.pc` no encontrado (`glib-sys`).

## Instalacion de dependencias

Usar el script del repo:

```bash
bash scripts/install-rustdesk-ubuntu-deps.sh
```

El script instala los paquetes base recomendados por RustDesk para Ubuntu/Debian (GTK, GLib, X11, Pulse, GStreamer, PAM, clang/cmake/ninja, etc.).

Si prefieres comando directo:

```bash
sudo apt-get update
sudo apt-get install -y \
  zip g++ gcc git curl wget nasm yasm pkg-config \
  libgtk-3-dev libglib2.0-dev clang libclang-dev \
  libxcb-randr0-dev libxdo-dev libxfixes-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libasound2-dev libpulse-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libpam0g-dev cmake make ninja-build
```

## Verificacion

Con script (recomendado):

```bash
bash scripts/check-rustdesk-fork.sh
```

Si necesitas pasar flags extra a `cargo check`, puedes agregarlos al final:

```bash
bash scripts/check-rustdesk-fork.sh --all-targets
```

Modo manual:

```bash
git -C /home/choy/Escritorio/rustdesk submodule update --init --recursive
cd /home/choy/Escritorio/rustdesk
~/.cargo/bin/cargo check
```

## Nota operativa

La instalacion requiere privilegios `sudo`. Si el entorno solicita contrasena interactiva, ejecuta el script en tu terminal local y escribe tu password cuando lo pida.
