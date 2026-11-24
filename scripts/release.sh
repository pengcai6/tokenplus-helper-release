#!/bin/bash
# 打包并发布到 GitHub Releases（pengcai6/tokenplus-helper-release）。
# 用户安装：curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/install.sh | bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
REPO="pengcai6/tokenplus-helper-release"
VERSION=$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
TAG="v$VERSION"
ASSET="dist/TokenPlus-Helper-macOS.dmg"

./scripts/build-dmg.sh
cp "dist/TokenPlus小助手-$VERSION.dmg" "$ASSET"

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "$TAG" "$ASSET" --repo "$REPO" --clobber
else
  gh release create "$TAG" "$ASSET" --repo "$REPO" --title "TokenPlus 小助手 $VERSION" \
    --notes "macOS（Apple 芯片）。终端一键安装：\`curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash\`"
fi
echo "已发布 ${TAG}：https://github.com/$REPO/releases/tag/$TAG"
