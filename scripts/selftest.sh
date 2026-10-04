#!/usr/bin/env bash
# 快速回归：debug 构建 + 配置自检 + 分享链接解析 + 全链路冒烟测试
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_HOME="$ROOT/.toolchain/cargo"
export RUSTUP_HOME="$ROOT/.toolchain/rustup"

TC_BIN="$(ls -d "$ROOT"/.toolchain/rustup/toolchains/*/bin 2>/dev/null | head -1)"
MINGW_BIN="$ROOT/.toolchain/mingw64/bin"
export PATH="$TC_BIN:$MINGW_BIN:$PATH"

cd "$ROOT/src-tauri"
echo "==> cargo build (debug，比 release 快很多)"
cargo build 2>&1 | grep -v "^\s*Compiling " | tail -30

BIN="$ROOT/src-tauri/target/debug/mvpn.exe"

CORE="$ROOT/.toolchain/downloads/sb/sing-box-1.14.2-windows-amd64/sing-box.exe"
if [ -f "$CORE" ]; then
  export MVP_CORE="$(cygpath -w "$CORE" 2>/dev/null || echo "$CORE")"
else
  echo "!! 未找到内核，跳过依赖内核的自检"
fi

rc=0
echo ""
echo "==> 1/2 配置生成校验 + 分享链接解析"
"$BIN" --selftest || rc=1
echo ""
echo "==> 2/2 全链路冒烟测试（生成配置 → 拉起内核 → 端口就绪 → 真实转发）"
"$BIN" --smoke || rc=1

echo ""
if [ "$rc" = "0" ]; then echo "==> 🎉 全部通过"; else echo "==> ❌ 存在失败项"; fi
exit $rc
