PROVIDER_ID='tokenplus'

# 去掉本工具写过的模型段（id 以 tokenplus 开头，含桌面版自动生成的子段；旧版 yituo-grok 段一并清掉），
# 把 [models] 的 default 换成新值，最后追加新的模型段。
write_grok_toml() {
  target="$1"
  default_id="$2"
  sections="$3"
  mkdir -p "$(dirname "$target")"
  tmp="$target.tokenplus-$$.tmp"
  if [ -f "$target" ]; then
    cp "$target" "$target.bak-$STAMP"
  else
    : >"$target"
  fi
  # 多行内容走环境变量，macOS 自带的 awk 不接受带换行的 -v
  # 空行先攒着，遇到要保留的内容才补上，反复运行不会越堆越多
  if ! TP_SECTIONS="$sections" awk -v def="$default_id" '
    function emit(line) { while (blanks > 0) { print ""; blanks-- } print line }
    /^[ \t]*$/ { if (!skip) blanks++; next }
    /^[ \t]*\[/ {
      skip = ($0 ~ /^[ \t]*\[[ \t]*model[ \t]*\.[ \t]*"?(tokenplus|yituo-grok)/)
      in_models = ($0 ~ /^[ \t]*\[[ \t]*models[ \t]*\][ \t]*(#.*)?$/)
      if (skip) next
      emit($0)
      if (in_models) { print "default = \"" def "\""; found = 1 }
      next
    }
    skip { next }
    in_models && /^[ \t]*default[ \t]*=/ { next }
    { emit($0) }
    END {
      if (!found) printf "\n[models]\ndefault = \"%s\"\n", def
      printf "\n%s", ENVIRON["TP_SECTIONS"]
    }
  ' "$target" >"$tmp"; then
    rm -f "$tmp"
    red "无法改写 ${target}，未做修改"
    exit 1
  fi
  mv "$tmp" "$target"
  chmod 600 "$target"
}

tp_banner 'Grok 一键配置'

tp_require_key
tp_require_model
BASE_URL="$TP_GATEWAY/v1"
MODEL_DEFAULT="$MODEL"

# ---------- 命令行版 Grok Build ----------
PATH="$PATH:$HOME/.local/bin:$HOME/.grok/bin"
CLI_BIN="$(command -v grok 2>/dev/null || true)"
if [ -n "$CLI_BIN" ]; then
  green "Grok Build 已安装：$CLI_BIN"
elif [ -n "${TOKENPLUS_NO_INSTALL:-}" ]; then
  yellow '没有检测到 grok 命令，已按要求跳过安装，配置仍会写好'
else
  step '正在用 xAI 官方脚本安装 Grok Build...'
  if curl -fsSL https://x.ai/cli/install.sh | bash; then
    CLI_BIN="$(command -v grok 2>/dev/null || true)"
  fi
  if [ -n "$CLI_BIN" ]; then
    green "已安装到 $CLI_BIN"
  else
    yellow 'Grok Build 安装失败（可能需要代理访问 x.ai），配置仍会写好'
    yellow '稍后可手动运行：curl -fsSL https://x.ai/cli/install.sh | bash'
  fi
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
CLI_SECTIONS=''
APP_MODELS=''
for name in $MODELS; do
  CLI_SECTIONS="$CLI_SECTIONS[model.\"$PROVIDER_ID/$name\"]
model = \"$name\"
name = \"TokenPlus · $name\"
base_url = \"$BASE_URL\"
api_key = \"$API_KEY\"
api_backend = \"responses\"
supports_reasoning_effort = true

"
  APP_MODELS="$APP_MODELS${APP_MODELS:+,}{\"id\":\"$name\",\"name\":\"$name\"}"
done

step '正在写入 Grok Build 配置...'
CLI_CONFIG="${GROK_HOME:-$HOME/.grok}/config.toml"
write_grok_toml "$CLI_CONFIG" "$PROVIDER_ID/$MODEL_DEFAULT" "$CLI_SECTIONS"
green "已写入 $CLI_CONFIG"

# ---------- Grok 桌面版 ----------
if [ -n "${GROK_APP_HOME:-}" ]; then
  APP_HOME="$GROK_APP_HOME"
elif [ "$(uname -s)" = Darwin ]; then
  APP_HOME="$HOME/Library/Application Support/com.grokapp.grok-app"
elif [ -d "$HOME/.grok-app" ]; then
  APP_HOME="$HOME/.grok-app"
else
  APP_HOME="${XDG_DATA_HOME:-$HOME/.local/share}/grok-app"
fi
APP_SECTION="[model.$PROVIDER_ID]
model = \"$MODEL_DEFAULT\"
name = \"TokenPlus · Grok\"
base_url = \"$BASE_URL\"
api_key = \"$API_KEY\"
api_backend = \"responses\"
supports_reasoning_effort = true
app_provider_mode = \"generic\"
app_models = '[$APP_MODELS]'
"
step '正在写入 Grok 桌面版配置...'
APP_CONFIG="$APP_HOME/agent-home/config.toml"
write_grok_toml "$APP_CONFIG" "$PROVIDER_ID" "$APP_SECTION"
green "已写入 $APP_CONFIG"

tp_probe responses

printf '\n'
printf '\033[32m  配置完成，默认模型 %s\033[0m\n' "$MODEL_DEFAULT"
printf '  命令行：新开一个终端，敲 grok（/model 可切换其他模型）\n'
printf '  桌面版：完全退出 Grok 后重新打开，在模型列表里选「TokenPlus · Grok」\n'
