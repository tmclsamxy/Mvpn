#!/usr/bin/env bash
# 打包：构建 + 集成 WebView2Loader + 内置 sing-box 内核 + 输出可分发目录
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT/scripts/build.sh"

VERSION="$(grep -m1 '^version' "$ROOT/src-tauri/Cargo.toml" | cut -d'"' -f2)"
OUT="$ROOT/dist/Mvpn-$VERSION"
# 注意：这里不做 rm -rf（清理操作在受限环境可能被拦），逐文件覆盖即可
mkdir -p "$OUT/core"

cp "$ROOT/src-tauri/target/release/mvpn.exe" "$OUT/Mvpn.exe"

# --- WebView2Loader.dll ---
# Rust 的 x86_64-pc-windows-gnu 目标是「动态」链接 WebView2Loader（MSVC 目标为静态链接），
# 缺失时 exe 会以退出码 127 直接启动失败。必须与 exe 放在同一目录。
WV="$(ls "$ROOT"/.toolchain/cargo/registry/src/*/webview2-com-sys-*/x64/WebView2Loader.dll 2>/dev/null | head -1 || true)"
if [ -n "$WV" ] && [ -f "$WV" ]; then
  cp "$WV" "$OUT/WebView2Loader.dll"
  echo "==> 已集成 WebView2Loader.dll"
else
  echo "!! 警告：未找到 WebView2Loader.dll，程序将无法启动！" >&2
fi

# --- 内置 sing-box 内核 ---
SB_DIR="$ROOT/.toolchain/downloads/sb/sing-box-1.14.2-windows-amd64"
if [ -d "$SB_DIR" ]; then
  cp "$SB_DIR/sing-box.exe" "$OUT/core/" 2>/dev/null || true
  cp "$SB_DIR/libcronet.dll" "$OUT/core/" 2>/dev/null || true
  echo "==> 已内置 sing-box 内核"
else
  echo "!! 未找到已解压内核，首次运行时请点击「下载/更新内核」"
fi

cp "$ROOT/README.md" "$OUT/" 2>/dev/null || true

echo ""
echo "==> 打包完成: $OUT"
du -sh "$OUT"
ls -la "$OUT"
