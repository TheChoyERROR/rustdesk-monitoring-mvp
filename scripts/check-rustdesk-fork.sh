#!/usr/bin/env bash
set -euo pipefail

RUSTDESK_DIR="${RUSTDESK_DIR:-$HOME/Escritorio/rustdesk}"
DEPS_ROOT="${RUSTDESK_DEPS_ROOT:-$HOME/.local/opt/rustdesk-deps}"
CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin/cargo}"
CC_BIN="${CC_BIN:-/usr/bin/cc}"
CXX_BIN="${CXX_BIN:-/usr/bin/c++}"
PKG_DIR="$DEPS_ROOT/usr/lib/x86_64-linux-gnu/pkgconfig"

if [ ! -d "$RUSTDESK_DIR" ]; then
  echo "No existe RUSTDESK_DIR: $RUSTDESK_DIR"
  echo "Exporta RUSTDESK_DIR o clona RustDesk en $HOME/Escritorio/rustdesk"
  exit 1
fi

if [ ! -d "$PKG_DIR" ]; then
  echo "No existe el arbol de dependencias local: $DEPS_ROOT"
  echo "Primero instala dependencias y extrae paquetes (docs/rustdesk-fork-setup-ubuntu.md)."
  exit 1
fi

if [ ! -x "$CARGO_BIN" ]; then
  echo "No se encontro cargo en: $CARGO_BIN"
  exit 1
fi

if [ ! -x "$CC_BIN" ] || [ ! -x "$CXX_BIN" ]; then
  echo "Compilador no encontrado. CC=$CC_BIN CXX=$CXX_BIN"
  exit 1
fi

# Ubuntu no siempre provee libyuv.pc; lo creamos para resolver via pkg-config.
if [ ! -f "$PKG_DIR/libyuv.pc" ]; then
  cat > "$PKG_DIR/libyuv.pc" <<'EOF'
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib/x86_64-linux-gnu
includedir=${prefix}/include

Name: libyuv
Description: YUV conversion and scaling functionality
Version: 0.0.1883
Libs: -L${libdir} -lyuv
Cflags: -I${includedir}
EOF
fi

export CC="$CC_BIN"
export CXX="$CXX_BIN"
export PKG_CONFIG_PATH="$PKG_DIR${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export PKG_CONFIG_SYSROOT_DIR="$DEPS_ROOT"
export OPENSSL_DIR="$DEPS_ROOT/usr"
export OPENSSL_LIB_DIR="$DEPS_ROOT/usr/lib/x86_64-linux-gnu"
export OPENSSL_INCLUDE_DIR="$DEPS_ROOT/usr/include"
export CFLAGS="-I$DEPS_ROOT/usr/include -I$DEPS_ROOT/usr/include/x86_64-linux-gnu${CFLAGS:+ $CFLAGS}"

echo "Ejecutando cargo check en $RUSTDESK_DIR"
cd "$RUSTDESK_DIR"
"$CARGO_BIN" check --features linux-pkg-config "$@"
