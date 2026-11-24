#!/bin/bash
# 打包 TokenPlus 小助手 DMG。
#
# 两种模式：
#   1) 正式分发（推荐）：设置 SIGN_IDENTITY + NOTARY_PROFILE，自动 Developer ID 签名、
#      Apple 公证、装订票据，用户双击直接打开，不会出现「仍要打开」。
#        SIGN_IDENTITY="Developer ID Application: 公司名 (TEAMID)" \
#        NOTARY_PROFILE=tokenplus-notary ./scripts/build-dmg.sh
#      NOTARY_PROFILE 先用一次：
#        xcrun notarytool store-credentials tokenplus-notary \
#          --apple-id 你的AppleID --team-id TEAMID --password App专用密码
#   2) 本地 / 内测：不设置任何变量，ad-hoc 签名，DMG 内附「安装说明.txt」。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION=$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
NAME="TokenPlus 小助手"
APP="src-tauri/target/release/bundle/macos/$NAME.app"
STAGE="dist/dmg-stage"
DMG="dist/TokenPlus小助手-$VERSION.dmg"
SIGN_IDENTITY="${SIGN_IDENTITY:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"

if [ -n "$SIGN_IDENTITY" ]; then
  security find-identity -v -p codesigning | grep -qF "$SIGN_IDENTITY" \
    || { echo "钥匙串里找不到签名证书：$SIGN_IDENTITY"; exit 1; }
  case "$SIGN_IDENTITY" in
    "Developer ID Application"*) ;;
    *) echo "公证需要「Developer ID Application」证书，当前是：$SIGN_IDENTITY"; exit 1 ;;
  esac
  [ -n "$NOTARY_PROFILE" ] || { echo "请设置 NOTARY_PROFILE（见脚本开头说明）"; exit 1; }
  # tauri 用该证书签名并开启 hardened runtime
  npm run tauri build -- --bundles app \
    --config "{\"bundle\":{\"macOS\":{\"signingIdentity\":\"$SIGN_IDENTITY\"}}}"
  codesign --verify --deep --strict "$APP"

  echo "== 公证 .app"
  ZIP="dist/notarize-app.zip"
  rm -f "$ZIP" && ditto -c -k --keepParent "$APP" "$ZIP"
  xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  rm -f "$ZIP"
else
  npm run tauri build -- --bundles app
  codesign --force --deep --sign - "$APP"
  codesign --verify --deep --strict "$APP"
fi

rm -rf "$STAGE" && mkdir -p "$STAGE"
ditto "$APP" "$STAGE/$NAME.app"
ln -s /Applications "$STAGE/Applications"
if [ -z "$SIGN_IDENTITY" ]; then
  cp "scripts/安装说明.txt" "$STAGE/"
fi
rm -f "$DMG"
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"

if [ -n "$SIGN_IDENTITY" ]; then
  echo "== 签名并公证 DMG"
  codesign --force --sign "$SIGN_IDENTITY" --timestamp "$DMG"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
  spctl -a -vv -t install "$DMG"
  spctl -a -vv "$APP"
fi
echo "DMG: $ROOT/$DMG"
