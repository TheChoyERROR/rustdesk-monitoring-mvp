#!/usr/bin/env bash
set -euo pipefail

echo "[1/3] Actualizando indice de paquetes..."
sudo apt-get update

echo "[2/3] Instalando dependencias nativas para compilar RustDesk..."
sudo apt-get install -y \
  zip g++ gcc git curl wget nasm yasm pkg-config \
  libgtk-3-dev libglib2.0-dev clang libclang-dev \
  libxcb-randr0-dev libxdo-dev libxfixes-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libasound2-dev libpulse-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libpam0g-dev cmake make ninja-build

RUSTDESK_DIR="${RUSTDESK_DIR:-$HOME/Escritorio/rustdesk}"
if [ -d "$RUSTDESK_DIR/.git" ]; then
  echo "[3/3] Inicializando submodulos del fork en $RUSTDESK_DIR..."
  git -C "$RUSTDESK_DIR" submodule update --init --recursive
else
  echo "[3/3] Omitido: no se encontro un repo git en $RUSTDESK_DIR"
fi

echo "Listo. Siguiente validacion recomendada:"
echo "  cd \"$RUSTDESK_DIR\" && ~/.cargo/bin/cargo check"
