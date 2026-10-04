#!/usr/bin/env bash
# Mvpn 构建脚本（Windows / Git Bash）
# 使用项目内置的便携 Rust(GNU) + MinGW-w64 工具链，不依赖系统环境
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_HOME="$ROOT/.toolchain/cargo"
export RUSTUP_HOME="$ROOT/.toolchain/rustup"

# 定位工具链的实体二进制目录（避免 rustup shim 在部分 shell 下无输出）
TC_BIN="$(ls -d "$ROOT"/.toolchain/rustup/toolchains/*/bin 2>/dev/null | head -1)"
if [ -z "$TC_BIN" ]; then
  TC_BIN="$CARGO_HOME/bin"
fi

# 定位 MinGW（winlibs 解压后可能是 mingw64/ 或含版本号的目录）
MINGW_BIN=""
for d in "$ROOT"/.toolchain/mingw64 "$ROOT"/.toolchain/mingw*/mingw64 "$ROOT"/.toolchain/mingw*; do
  if [ -f "$d/bin/gcc.exe" ]; then
    MINGW_BIN="$d/bin"; break
  fi
done
if [ -z "$MINGW_BIN" ]; then
  echo "!! 未找到 MinGW，请先运行 scripts/setup-toolchain.sh" >&2
  exit 1
fi

export PATH="$TC_BIN:$MINGW_BIN:$PATH"
echo "==> cargo: $(cargo --version)"
echo "==> gcc  : $(gcc --version | head -1)"
echo "==> target: $(rustc -vV | grep host)"

cd "$ROOT/src-tauri"
cargo build --release "$@"

OUT="$ROOT/src-tauri/target/release/mvpn.exe"
if [ -f "$OUT" ]; then
  echo ""
  echo "==> 构建成功: $OUT  ($(du -h "$OUT" | cut -f1))"
else
  echo "!! 未找到产物" >&2
  exit 1
fi
