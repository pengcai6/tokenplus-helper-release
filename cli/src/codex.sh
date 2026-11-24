BEGIN_BASE='# >>> tokenplus codex setup (base) >>>'
END_BASE='# <<< tokenplus codex setup (base) <<<'
BEGIN_PROVIDER='# >>> tokenplus codex setup (provider) >>>'
END_PROVIDER='# <<< tokenplus codex setup (provider) <<<'
PROVIDER_ID='tokenplus'
PROVIDER_NAME='TokenPlus'


# 桌面客户端的位置，找不到就当没装，界面语言那步整段跳过。
codex_app_path() {
  local app bundle_id
  for app in '/Applications/Codex.app' "$HOME/Applications/Codex.app" \
    '/Applications/ChatGPT.app' "$HOME/Applications/ChatGPT.app"; do
    [ -d "$app" ] || continue
    bundle_id="$(app_bundle_id "$app")" || continue
    # Codex has also shipped in a bundle named ChatGPT.app. The classic
    # ChatGPT client uses com.openai.chat and must not be modified or quit.
    case "$bundle_id" in
      com.openai.codex|com.openai.codex.alpha|com.openai.codex.beta)
        printf '%s' "$app"
        return 0
        ;;
    esac
  done
  return 1
}

# Codex 同时只允许一个实例。只向实际选中的 bundle 发出退出请求，随后按
# 可执行文件精确匹配进程；不使用 awk 正则，也不强杀。应用主进程退出后，
# Electron 的 renderer/helper 子进程也会随其一起结束。
app_bundle_id() {
  /usr/bin/plutil -extract CFBundleIdentifier raw -o - "$1/Contents/Info.plist" 2>/dev/null
}

stop_codex_app() {
  local locale_app_to_stop="$1" locale_bundle_id
  locale_bundle_id="$(app_bundle_id "$locale_app_to_stop")" || return 1
  [ -n "$locale_bundle_id" ] || return 1
  # 先判断是否在运行：AppleScript 向没运行的应用发命令时会先把它启动起来，
  # 而 `is running` 不会（AppleScript 2.0 / macOS 10.5 起）。
  /usr/bin/osascript \
    -e "if application id \"$locale_bundle_id\" is running then" \
    -e "tell application id \"$locale_bundle_id\" to quit" \
    -e 'end if' >/dev/null 2>&1 || true
}

# 输出仍在运行且确实属于指定 app/profile 的 PID。shell case 使用字面量匹配，
# 因此路径中的 ., [, / 等字符不会变成 awk/grep 正则，也不会匹配检查命令自身。
profile_process_pids() {
  local locale_app_to_check="$1" locale_executable_name locale_executable_path
  local locale_process_list locale_process_line locale_process_pid locale_process_command
  locale_executable_name="$(/usr/bin/plutil -extract CFBundleExecutable raw -o - "$locale_app_to_check/Contents/Info.plist" 2>/dev/null)" || return 1
  [ -n "$locale_executable_name" ] || return 1
  locale_executable_path="$locale_app_to_check/Contents/MacOS/$locale_executable_name"
  # `comm` is only a basename on some macOS releases. `command` retains the
  # executable path, which lets us distinguish this bundle from another
  # Codex/ChatGPT installation without relying on a fragile name-only match.
  locale_process_list="$(/bin/ps -axww -o pid=,command= 2>/dev/null)" || return 1
  [ -n "$locale_process_list" ] || return 1
  while IFS= read -r locale_process_line; do
    locale_process_line="${locale_process_line#"${locale_process_line%%[![:space:]]*}"}"
    locale_process_pid="${locale_process_line%%[[:space:]]*}"
    locale_process_command="${locale_process_line#"$locale_process_pid"}"
    locale_process_command="${locale_process_command#"${locale_process_command%%[![:space:]]*}"}"
    [ -n "$locale_process_pid" ] || continue
    case "$locale_process_command" in
      "$locale_executable_path"|"$locale_executable_path "*|"$locale_app_to_check/Contents/Frameworks/"*) printf '%s\n' "$locale_process_pid" ;;
    esac
  done <<< "$locale_process_list"
}

wait_for_codex_profile_exit() {
  local locale_app_to_wait="$1" locale_profile_waited=0 locale_profile_pids
  while [ "$locale_profile_waited" -lt 40 ]; do
    locale_profile_pids="$(profile_process_pids "$locale_app_to_wait")" || return 1
    if [ -z "$locale_profile_pids" ]; then return 0; fi
    /bin/sleep 0.5
    locale_profile_waited=$((locale_profile_waited + 1))
  done
  return 1
}

tp_banner 'Codex 一键配置'

tp_require_key
tp_require_model
BASE_URL="$TP_GATEWAY/v1"

CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
CONFIG_PATH="$CODEX_DIR/config.toml"
mkdir -p "$CODEX_DIR"

# 先退出桌面客户端，避免旧进程继续使用模型目录或回写缓存。
if CODEX_APP="$(codex_app_path)"; then
  step '正在关闭 Codex 以刷新模型配置...'
  # 顶层受 set -e 约束：读不到 bundle id 时别让脚本在写配置之前悄悄退出。
  stop_codex_app "$CODEX_APP" || true
  if ! wait_for_codex_profile_exit "$CODEX_APP"; then
    yellow 'Codex 未能自动退出，配置照常写入；请手动退出并重新打开 Codex'
  fi
fi

step '正在写入 Codex 配置...'
STAMP="$(date +%Y%m%d-%H%M%S)"
while [ -e "$CONFIG_PATH.bak-$STAMP" ] || [ -e "$CODEX_DIR/models_cache.json.bak-$STAMP" ]; do
  STAMP="$STAMP-1"
done
EXISTING=''
if [ -f "$CONFIG_PATH" ]; then
  cp "$CONFIG_PATH" "$CONFIG_PATH.bak-$STAMP"
  EXISTING="$(cat "$CONFIG_PATH")"
fi

# 去掉本工具上次写入的两段、去掉同名 provider 表、注释掉会冲突的顶层键，
# 再把首尾空行修掉，保证重复运行结果稳定。
KEPT="$(printf '%s\n' "$EXISTING" | awk \
  -v begin_base="$BEGIN_BASE" -v end_base="$END_BASE" \
  -v begin_provider="$BEGIN_PROVIDER" -v end_provider="$END_PROVIDER" \
  -v provider_header="[model_providers.$PROVIDER_ID]" '
{
  line = $0
  s = line
  sub(/^[ \t]+/, "", s)
  sub(/[ \t]+$/, "", s)
}
s == begin_base || s ~ /^# >>> (yituohub|zhishuhub) codex setup \(base\) >>>$/ { in_base = 1; next }
s == end_base || s ~ /^# <<< (yituohub|zhishuhub) codex setup \(base\) <<<$/ { in_base = 0; next }
s == begin_provider || s ~ /^# >>> (yituohub|zhishuhub) codex setup \(provider\) >>>$/ { in_provider = 1; next }
s == end_provider || s ~ /^# <<< (yituohub|zhishuhub) codex setup \(provider\) <<<$/ { in_provider = 0; next }
in_base || in_provider { next }
s == provider_header { in_table = 1; next }
in_table && s ~ /^\[/ { in_table = 0 }
in_table { next }
s ~ /^\[/ { seen_table = 1 }
# 停用 CC Switch 等工具写入的旧模型目录和默认 profile；保留表定义供用户切回。
!seen_table && (s ~ /^(model|model_provider|model_reasoning_effort|model_catalog_json|profile|approval_policy|sandbox_mode)[ \t]*=/) {
  kept[++n] = "# " line "    # 由 TokenPlus 配置工具接管"
  next
}
{ kept[++n] = line }
END {
  first = 1
  while (first <= n && kept[first] ~ /^[ \t]*$/) first++
  last = n
  while (last >= first && kept[last] ~ /^[ \t]*$/) last--
  for (i = first; i <= last; i++) print kept[i]
}
')"

{
  printf '%s\n' "$BEGIN_BASE"
  printf 'model = "%s"\n' "$MODEL"
  printf 'model_provider = "%s"\n' "$PROVIDER_ID"
  printf 'model_reasoning_effort = "high"\n'
  printf 'approval_policy = "never"\n'
  printf 'sandbox_mode = "danger-full-access"\n'
  printf '%s\n' "$END_BASE"
  if [ -n "$KEPT" ]; then
    printf '\n%s\n' "$KEPT"
  fi
  printf '\n%s\n' "$BEGIN_PROVIDER"
  printf '[model_providers.%s]\n' "$PROVIDER_ID"
  printf 'name = "%s"\n' "$PROVIDER_NAME"
  printf 'base_url = "%s"\n' "$BASE_URL"
  printf 'wire_api = "responses"\n'
  printf 'experimental_bearer_token = "%s"\n' "$API_KEY"
  printf 'requires_openai_auth = false\n'
  printf 'http_headers = { "x-openai-actor-authorization" = "tokenplus" }\n'
  printf '%s\n' "$END_PROVIDER"
} > "$CONFIG_PATH"
# Language selection must not depend on downloading the optional network helper.
locale_tmp="$CONFIG_PATH.locale-$$"
awk '
function write_locale() { print "localeOverride = \"zh-CN\""; written = 1 }
{
  s = $0
  sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s)
  if (s ~ /^\[/) {
    if (inside && !written) write_locale()
    inside = (s == "[desktop]")
    if (inside) seen = 1
    else if (!seen && !written && s ~ /^\[desktop\./) {
      print "[desktop]"; write_locale(); print ""; seen = 1
    }
  }
  if (inside && s ~ /^localeOverride[ \t]*=/) {
    if (!written) write_locale()
    next
  }
  print
}
END {
  if (inside && !written) write_locale()
  if (!seen) { print ""; print "[desktop]"; write_locale() }
}
' "$CONFIG_PATH" > "$locale_tmp"
mv "$locale_tmp" "$CONFIG_PATH"
chmod 600 "$CONFIG_PATH"
green "已写入 $CONFIG_PATH"
green '已设置桌面界面语言为中文，重启 Codex 后生效'
green '已设置完全访问权限，重启 Codex 后生效'

if [ -f "$CODEX_DIR/models_cache.json" ]; then
  mv "$CODEX_DIR/models_cache.json" "$CODEX_DIR/models_cache.json.bak-$STAMP"
  green '旧模型列表已备份，重启后会重新加载'
fi

HAS_DESKTOP=0
if codex_app_path >/dev/null; then
  HAS_DESKTOP=1
fi
HAS_CLI=0
if command -v codex >/dev/null 2>&1; then
  HAS_CLI=1
fi

printf '\n'
printf '\033[32m  配置完成，主模型 %s\033[0m\n' "$MODEL"
if [ "$HAS_DESKTOP" = "1" ]; then
  printf '  桌面客户端：重启 Codex 后即可开始使用\n'
fi
if [ "$HAS_CLI" = "1" ]; then
  printf '  命令行：直接运行 codex\n'
fi
if [ "$HAS_DESKTOP" = "0" ] && [ "$HAS_CLI" = "0" ]; then
  printf '  装好 Codex（桌面客户端或命令行版）后直接可用，不用再运行本工具\n'
fi
if [ "$HAS_DESKTOP" = "1" ]; then
  printf '\n'
  printf '  桌面客户端的会话列表按供应商分开存放，\n'
  printf '  官方账号下的会话切回官方账号后照常可见，不会丢失。\n'
fi
