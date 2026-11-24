tp_banner 'WorkBuddy 一键配置'

# 使用已安装的 Node.js 或 WorkBuddy 自带的运行时
runtime=$(command -v node || true)
if [ -n "$runtime" ] && ! "$runtime" -e 'if (Number(process.versions.node.split(".")[0]) < 20) process.exit(1)' >/dev/null 2>&1; then
  runtime=''
fi
electron_runtime=0
if [ -z "$runtime" ]; then
  for app in '/Applications/WorkBuddy.app' "$HOME/Applications/WorkBuddy.app" '/Applications/WorkBuddy AI.app' "$HOME/Applications/WorkBuddy AI.app"; do
    if [ -x "$app/Contents/MacOS/Electron" ]; then runtime="$app/Contents/MacOS/Electron"; electron_runtime=1; break; fi
    if [ -x "$app/Contents/MacOS/WorkBuddy" ]; then runtime="$app/Contents/MacOS/WorkBuddy"; electron_runtime=1; break; fi
  done
fi
[ -n "$runtime" ] || { red '请先安装 WorkBuddy，或安装 Node.js 20 及以上版本后重试'; exit 1; }

tp_require_key

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tp_fetch_asset workbuddy-config.mjs "$tmp/workbuddy-config.mjs" || { red '配置工具下载失败，请检查网络后重试'; exit 1; }

step '正在把 TokenPlus 的模型写入 WorkBuddy...'
if [ "$electron_runtime" = 1 ]; then
  ELECTRON_RUN_AS_NODE=1 TOKENPLUS_API_KEY="$API_KEY" TOKENPLUS_MODELS="$MODELS" \
    TOKENPLUS_BASE_URL="$TP_GATEWAY/v1" "$runtime" "$tmp/workbuddy-config.mjs" || { red 'WorkBuddy 配置未完成'; exit 1; }
else
  TOKENPLUS_API_KEY="$API_KEY" TOKENPLUS_MODELS="$MODELS" \
    TOKENPLUS_BASE_URL="$TP_GATEWAY/v1" "$runtime" "$tmp/workbuddy-config.mjs" || { red 'WorkBuddy 配置未完成'; exit 1; }
fi

printf '\n'
printf '\033[32m  配置完成\033[0m\n'
printf '  完全退出并重新打开 WorkBuddy，在模型选择器里选 TokenPlus 下的模型\n'
