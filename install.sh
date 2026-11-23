#!/bin/bash
# TokenPlus 小助手 —— macOS 一键安装
#   curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/install.sh | bash
set -euo pipefail

REPO="pengcai6/tokenplus-helper-release"
ASSET="TokenPlus-Helper-macOS.dmg"
URL="https://github.com/$REPO/releases/latest/download/$ASSET"
NAME="TokenPlus 小助手"
DEST="/Applications/$NAME.app"

say() { printf '\033[1;35m==>\033[0m %s\n' "$1"; }
die() { printf '\033[1;31m安装失败：\033[0m%s\n' "$1" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "只支持 macOS"
[ "$(uname -m)" = "arm64" ] || die "目前只支持 Apple 芯片（M 系列）的 Mac"

TMP="$(mktemp -d)"
MNT="$TMP/mnt"
cleanup() {
  /usr/bin/hdiutil detach "$MNT" -quiet >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

say "下载 $NAME …"
curl -fL --progress-bar "$URL" -o "$TMP/app.dmg" || die "下载失败，请检查网络后重试"

say "安装到「应用程序」…"
mkdir -p "$MNT"
/usr/bin/hdiutil attach "$TMP/app.dmg" -nobrowse -readonly -quiet -mountpoint "$MNT" \
  || die "安装包无法打开，请重试"
[ -d "$MNT/$NAME.app" ] || die "安装包内容不完整"

if pgrep -f "$DEST/Contents/MacOS/" >/dev/null 2>&1; then
  osascript -e "tell application \"$NAME\" to quit" >/dev/null 2>&1 || true
  sleep 1
  pkill -f "$DEST/Contents/MacOS/" >/dev/null 2>&1 || true
fi

if [ -w /Applications ]; then
  rm -rf "$DEST"
  /usr/bin/ditto "$MNT/$NAME.app" "$DEST"
else
  say "需要管理员权限写入「应用程序」，请输入开机密码"
  sudo rm -rf "$DEST"
  sudo /usr/bin/ditto "$MNT/$NAME.app" "$DEST"
fi
/usr/bin/xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

say "安装完成，正在打开 $NAME"
open "$DEST"
