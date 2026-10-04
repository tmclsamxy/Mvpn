#!/usr/bin/env bash
# 一键安装便携构建工具链（免管理员）：Rust(GNU) + MinGW-w64
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DL="$ROOT/.toolchain/downloads"
mkdir -p "$DL"
PROXY="${MVPN_PROXY:-}"

curl_opt=(-sSL --retry 5 --retry-delay 2 --retry-all-errors)
[ -n "$PROXY" ] && curl_opt+=(-x "$PROXY")

export CARGO_HOME="$ROOT/.toolchain/cargo"
export RUSTUP_HOME="$ROOT/.toolchain/rustup"

# 1) Rust GNU 工具链
if [ ! -f "$CARGO_HOME/bin/cargo.exe" ]; then
  echo "==> 下载 rustup-init (x86_64-pc-windows-gnu)"
  curl "${curl_opt[@]}" -o "$DL/rustup-init.exe" \
    "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe"
  "$DL/rustup-init.exe" -y --no-modify-path --profile minimal \
    --default-host x86_64-pc-windows-gnu --default-toolchain stable
fi

# 2) MinGW-w64（链接器）
if [ ! -d "$ROOT/.toolchain/mingw64" ]; then
  echo "==> 下载 winlibs MinGW-w64"
  curl "${curl_opt[@]}" -o "$DL/mingw.zip" \
    "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-14.0.0-ucrt-r2/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r2.zip"
  echo "==> 解压 MinGW"
  /c/Windows/System32/tar.exe -xf "$DL/mingw.zip" -C "$ROOT/.toolchain"
fi

echo "==> 完成。执行 scripts/build.sh 开始构建。"
