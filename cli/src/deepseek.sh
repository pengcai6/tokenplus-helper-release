tp_banner 'DeepSeek Harness 一键配置'

app=''
for p in "/Applications/DeepSeek Harness.app" "$HOME/Applications/DeepSeek Harness.app"; do
  if [ -f "$p/Contents/Resources/app.asar" ]; then app="$p/Contents/Resources/app.asar"; break; fi
  if [ -d "$p/Contents/Resources/app" ]; then app="$p/Contents/Resources/app"; break; fi
done
[ -n "$app" ] || { red '未找到 DeepSeek Harness，请先安装桌面版'; exit 1; }
exe="$(dirname "$(dirname "$app")")/MacOS/DeepSeek Harness"; [ -x "$exe" ] || exe="$(dirname "$(dirname "$app")")/MacOS/deepseek-harness-desktop"
[ -x "$exe" ] || { red '未找到 DeepSeek Harness 可执行文件'; exit 1; }

tp_require_key
tp_require_model

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
# macOS may expose /private/var through /var; use the same physical path as ESM.
tmp=$(cd "$tmp" && pwd -P)
tp_fetch_asset deepseek-config.mjs "$tmp/deepseek-config.mjs" || { red '配置工具下载失败，请检查网络后重试'; exit 1; }

step '正在写入 DeepSeek Harness 配置...'
# 用应用自带的运行时和 YAML 库改写配置；key 走环境变量，不出现在进程参数里
ELECTRON_RUN_AS_NODE=1 TOKENPLUS_API_KEY="$API_KEY" TOKENPLUS_MODEL="$MODEL" \
  TOKENPLUS_MODELS="$MODELS" TOKENPLUS_BASE_URL="$TP_GATEWAY/v1" \
  "$exe" "$tmp/deepseek-config.mjs" "$app" || { red 'DeepSeek Harness 配置未完成'; exit 1; }
green '已写入 ~/.dsh/settings.yaml 和 ~/.dsh/.credentials.yaml'

printf '\n'
printf '\033[32m  配置完成，默认模型 %s\033[0m\n' "$MODEL"
printf '  完全退出 DeepSeek Harness 后重新打开，新建对话即可使用\n'
