#!/usr/bin/env bash
# 发布到 GitHub Releases：打包 zip → 生成版本清单 → 创建 Release → 上传产物
#
# 用法：
#   MVPN_REPO=owner/name GH_TOKEN=ghp_xxx ./scripts/publish.sh [版本号]
#
# 产物：
#   Release v<版本> 上附带 Mvpn-<版本>-win-x64.zip 与 latest.json
#   客户端「设置 → 客户端更新 → 版本清单地址」填：
#     https://github.com/<owner>/<repo>/releases/latest/download/latest.json
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

: "${GH_TOKEN:?请先设置 GH_TOKEN}"
: "${MVPN_REPO:?请先设置 MVPN_REPO，例如 yourname/Mvpn}"

VERSION="${1:-$(grep -m1 '^version' src-tauri/Cargo.toml | cut -d'"' -f2)}"
API="https://api.github.com"
ZIP="Mvpn-${VERSION}-win-x64.zip"

echo "==> 1/4 构建"
bash scripts/package.sh >/dev/null
DIST="$ROOT/dist/Mvpn-${VERSION}"
[ -d "$DIST" ] || { echo "打包目录不存在: $DIST" >&2; exit 1; }

echo "==> 2/4 生成 zip"
rm -f "$ROOT/dist/$ZIP"
/c/Windows/System32/tar.exe -a -c -f "$ROOT/dist/$ZIP" -C "$ROOT/dist" "Mvpn-${VERSION}"
ls -la "$ROOT/dist/$ZIP"

echo "==> 3/4 生成版本清单 latest.json"
# 稳定地址：GitHub 会把 releases/latest/download/<asset> 指向最新 Release 的同名资源
MANIFEST_URL="https://github.com/${MVPN_REPO}/releases/latest/download/latest.json"
cat > "$ROOT/dist/latest.json" <<EOF
{
  "version": "${VERSION}",
  "notes": "见 Release 说明",
  "url": "https://github.com/${MVPN_REPO}/releases/download/v${VERSION}/${ZIP}"
}
EOF
cat "$ROOT/dist/latest.json"

echo "==> 4/4 创建 Release 并上传"
AUTH=(-H "Authorization: Bearer $GH_TOKEN" -H "Accept: application/vnd.github+json")

# 已存在则删除重建，保证资产与清单一致
RID=$(curl -sS "${AUTH[@]}" "$API/repos/$MVPN_REPO/releases/tags/v${VERSION}" \
      | python -c "import json,sys;d=json.load(sys.stdin);print(d.get('id',''))" 2>/dev/null || true)
if [ -n "$RID" ]; then
  echo "    已存在 v${VERSION}，先删除以刷新资产"
  curl -sS -X DELETE "${AUTH[@]}" "$API/repos/$MVPN_REPO/releases/$RID" >/dev/null
  sleep 2
fi

RID=$(curl -sS -X POST "${AUTH[@]}" \
  -H "Content-Type: application/json" \
  "$API/repos/$MVPN_REPO/releases" \
  -d "{\"tag_name\":\"v${VERSION}\",\"name\":\"Mvpn ${VERSION}\",\"body\":\"Windows 轻量代理客户端（sing-box 内核）。\\n\\n客户端更新清单地址：\\n\\n${MANIFEST_URL}\\n\",\"draft\":false,\"prerelease\":false}" \
  | python -c "import json,sys;d=json.load(sys.stdin);print(d.get('id',''))")

if [ -z "$RID" ]; then
  echo "❌ 创建 Release 失败，请检查 MVPN_REPO / GH_TOKEN 权限（Contents: write）" >&2
  exit 1
fi
echo "    Release id=$RID"

for f in "$ZIP" latest.json; do
  echo "    上传 $f"
  curl -sS -X POST "${AUTH[@]}" \
    "https://uploads.github.com/repos/$MVPN_REPO/releases/$RID/assets?name=$f" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@$ROOT/dist/$f" \
    | python -c "import json,sys;d=json.load(sys.stdin);print('      ->',d.get('browser_download_url') or d.get('message'))" || true
done

echo ""
echo "✅ 发布完成"
echo "   清单地址（填进客户端设置）：$MANIFEST_URL"
echo "   下载页：https://github.com/$MVPN_REPO/releases/tag/v${VERSION}"
