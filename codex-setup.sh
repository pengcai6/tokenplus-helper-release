#!/usr/bin/env bash
# TokenPlus · Codex 一键配置（macOS）
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/codex-setup.sh | bash
#     自动打开 TokenPlus，登录后复制 API Key 粘贴到终端回车即可。
#   curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/codex-setup.sh | bash -s -- sk-你的key 模型名
#     直接带参数，不再提问。
#
# 配置文件写入前都会先备份，只改本工具负责的那几项。
# 本文件由 cli/build.sh 生成，请修改 cli/src/ 下的源文件。

set -euo pipefail
umask 077

TP_SITE='https://www.tokenplus.cn/'
TP_GATEWAY="${TOKENPLUS_GATEWAY:-https://api.tokenplus.cn}"
TP_GATEWAY="${TP_GATEWAY%/}"
TP_RAW="${TOKENPLUS_RAW:-https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main}"
API_KEY="${1:-${TOKENPLUS_API_KEY:-}}"
MODEL="${2:-${TOKENPLUS_MODEL:-}}"
MODELS="${TOKENPLUS_MODELS:-}"
TP_EMBED="${TOKENPLUS_EMBED:-}"

green() { printf '\033[32m  [完成] %s\033[0m\n' "$1"; }
yellow() { printf '\033[33m  [注意] %s\033[0m\n' "$1"; }
red() { printf '\033[31m  [失败] %s\033[0m\n' "$1"; }
step() { printf '  %s\n' "$1"; }

tp_banner() {
  [ -n "$TP_EMBED" ] && return 0
  printf '\n\033[1;38;5;141m  TokenPlus\033[0m\033[38;5;245m · %s\033[0m\n' "$1"
  printf '\033[38;5;240m  ─────────────────────────────────────────────\033[0m\n'
}

tp_has_tty() { (: </dev/tty) 2>/dev/null; }

# 菜单期间关掉终端回显并隐藏光标，避免连按方向键时把画面挤乱
TP_STTY=''
tp_ui_begin() {
  TP_STTY="$(stty -g </dev/tty 2>/dev/null || true)"
  stty -echo </dev/tty 2>/dev/null || true
  printf '\033[?25l' >/dev/tty
  trap 'tp_ui_end' EXIT
  trap 'tp_ui_end; exit 130' INT TERM
}
tp_ui_end() {
  [ -n "$TP_STTY" ] && stty "$TP_STTY" </dev/tty 2>/dev/null || true
  TP_STTY=''
  printf '\033[?25h' >/dev/tty 2>/dev/null || true
}

tp_key_valid() {
  case "$1" in sk-?*) ;; *) return 1 ;; esac
  case "$1" in *[!A-Za-z0-9_-]*) return 1 ;; esac
  return 0
}

# 本机地址不出网，允许 http，方便本地测试
case "$TP_GATEWAY" in
  https://* | http://localhost:* | http://127.0.0.1:*) ;;
  *) red '网关地址必须使用 HTTPS'; exit 1 ;;
esac
case "$TP_GATEWAY" in *\"* | *\\* | *\'* | *[[:space:]]*) red '网关地址无效'; exit 1 ;; esac

# 用 key 读取模型列表：成功时把模型 id 逐行写进 MODELS，返回 0；
# key 无效返回 2，网络等其他问题返回 1。
tp_fetch_models() {
  local resp status body
  resp="$(curl -sS --connect-timeout 10 --max-time 25 -w '\n%{http_code}' \
    "$TP_GATEWAY/v1/models" -H "Authorization: Bearer $API_KEY" 2>/dev/null || true)"
  status="$(printf '%s\n' "$resp" | tail -n 1)"
  body="$(printf '%s\n' "$resp" | sed '$d')"
  case "$status" in
    200) ;;
    401 | 403) return 2 ;;
    *) return 1 ;;
  esac
  # 只取 data 数组里每个对象的 id；模型名里只允许不会破坏配置文件的字符
  MODELS="$(printf '%s' "$body" | tr -d '\n' \
    | grep -o '"id"[[:space:]]*:[[:space:]]*"[^"]*"' \
    | sed 's/^"id"[[:space:]]*:[[:space:]]*"//; s/"$//' \
    | grep -E '^[A-Za-z0-9._:/-]+$' | awk '!seen[$0]++' || true)"
  [ -n "$MODELS" ] || return 1
}

# 没带 key：打开 TokenPlus，让用户登录后复制 key 粘贴回来
tp_require_key() {
  local tries=0 rc
  if [ -n "$API_KEY" ]; then
    tp_key_valid "$API_KEY" || { red 'API Key 格式无效，应以 sk- 开头'; exit 1; }
    if [ -z "$MODELS" ]; then
      step '正在验证 API Key...'
      rc=0; tp_fetch_models || rc=$?
      [ "$rc" = 2 ] && { red 'API Key 无效、已停用或已过期'; exit 1; }
      [ "$rc" = 0 ] || { red '连不上 TokenPlus 网关，请检查网络后重试'; exit 1; }
    fi
    return 0
  fi
  if ! tp_has_tty; then
    red '当前没有可交互的终端'
    step '请把 API Key 作为参数传入：... | bash -s -- sk-你的key'
    exit 1
  fi
  step '正在打开 TokenPlus：登录后创建或复制一个 API Key'
  if [ -z "${TOKENPLUS_NO_OPEN:-}" ]; then
    /usr/bin/open "$TP_SITE" >/dev/null 2>&1 || step "浏览器没有自动打开，请手动访问 $TP_SITE"
  fi
  while [ "$tries" -lt 3 ]; do
    tries=$((tries + 1))
    printf '\033[38;5;141m  ›\033[0m 粘贴 API Key 后回车（输入不会显示）：' >/dev/tty
    IFS= read -rs API_KEY </dev/tty || API_KEY=''
    printf '\n' >/dev/tty
    API_KEY="$(printf '%s' "$API_KEY" | tr -d '[:space:]')"
    if ! tp_key_valid "$API_KEY"; then
      yellow 'API Key 格式无效，应以 sk- 开头，请重新粘贴'
      continue
    fi
    step '正在验证 API Key...'
    rc=0; tp_fetch_models || rc=$?
    if [ "$rc" = 0 ]; then return 0; fi
    if [ "$rc" = 2 ]; then
      yellow 'API Key 无效、已停用或已过期，请重新粘贴'
    else
      red '连不上 TokenPlus 网关，请检查网络后重试'
      exit 1
    fi
  done
  red '多次输入无效，已退出'
  exit 1
}

tp_model_listed() {
  printf '%s\n' "$MODELS" | grep -Fxq -- "$1"
}

# 终端里选一个模型：↑↓ 移动，直接打字筛选，回车确认
tp_pick_model() {
  local all=() low=() shown=() line query='' cursor=0 top=0 rows=10 drawn=0 key rest i n q
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    all[${#all[@]}]="$line"
    low[${#low[@]}]="$(printf '%s' "$line" | tr '[:upper:]' '[:lower:]')"
  done <<EOF
$MODELS
EOF
  exec 3>/dev/tty
  tp_ui_begin
  while :; do
    shown=()
    q="$(printf '%s' "$query" | tr '[:upper:]' '[:lower:]')"
    i=0
    while [ "$i" -lt "${#all[@]}" ]; do
      case "${low[$i]}" in *"$q"*) shown[${#shown[@]}]="${all[$i]}" ;; esac
      i=$((i + 1))
    done
    n=${#shown[@]}
    [ "$cursor" -ge "$n" ] && cursor=$((n > 0 ? n - 1 : 0))
    [ "$cursor" -lt "$top" ] && top=$cursor
    [ "$cursor" -ge $((top + rows)) ] && top=$((cursor - rows + 1))
    [ "$drawn" -gt 0 ] && printf '\033[%dA' "$drawn" >&3
    printf '\r\033[J' >&3
    printf '\033[38;5;141m  ?\033[0m 选择模型 \033[38;5;245m（↑↓ 移动 · 打字筛选 · 回车确认）\033[0m\n' >&3
    printf '\033[38;5;245m    筛选：\033[0m%s\033[38;5;141m▏\033[0m\n' "$query" >&3
    drawn=2
    if [ "$n" = 0 ]; then
      printf '\033[38;5;245m    没有匹配的模型\033[0m\n' >&3
      drawn=$((drawn + 1))
    fi
    i=$top
    while [ "$i" -lt "$n" ] && [ "$i" -lt $((top + rows)) ]; do
      if [ "$i" = "$cursor" ]; then
        printf '\033[38;5;141m  ❯ %s\033[0m\n' "${shown[$i]}" >&3
      else
        printf '    %s\n' "${shown[$i]}" >&3
      fi
      drawn=$((drawn + 1))
      i=$((i + 1))
    done
    printf '\033[38;5;245m    %d / %d 个模型\033[0m\n' "$n" "${#all[@]}" >&3
    drawn=$((drawn + 1))

    IFS= read -rsn1 key </dev/tty || key=''
    case "$key" in
      '')
        if [ "$n" -gt 0 ]; then MODEL="${shown[$cursor]}"; break; fi
        ;;
      $'\033')
        rest=''
        IFS= read -rsn2 -t 1 rest </dev/tty || rest=''
        case "$rest" in
          '[A') [ "$cursor" -gt 0 ] && cursor=$((cursor - 1)) ;;
          '[B') [ "$cursor" -lt $((n - 1)) ] && cursor=$((cursor + 1)) ;;
        esac
        ;;
      $'\177' | $'\b') query="${query%?}"; cursor=0; top=0 ;;
      *)
        case "$key" in
          [[:print:]]) query="$query$key"; cursor=0; top=0 ;;
        esac
        ;;
    esac
  done
  printf '\033[%dA\r\033[J' "$drawn" >&3
  exec 3>&-
  tp_ui_end
  trap - EXIT INT TERM
}

tp_require_model() {
  if [ -n "$MODEL" ]; then
    case "$MODEL" in *[!A-Za-z0-9._:/-]*) red '模型名包含无效字符'; exit 1 ;; esac
    tp_model_listed "$MODEL" || { red "模型 $MODEL 不在这个 API Key 可用的列表里"; exit 1; }
    return 0
  fi
  if ! tp_has_tty; then
    red '当前没有可交互的终端，无法选择模型'
    step '请把模型名作为第二个参数传入：... | bash -s -- sk-你的key 模型名'
    exit 1
  fi
  tp_pick_model
  green "已选择模型 $MODEL"
}

# 取随脚本一起发布的辅助文件：本地有就用本地的，否则从仓库下载
tp_fetch_asset() {
  local name="$1" dest="$2"
  if [ -n "${TOKENPLUS_ASSET_DIR:-}" ] && [ -f "$TOKENPLUS_ASSET_DIR/$name" ]; then
    cp "$TOKENPLUS_ASSET_DIR/$name" "$dest"
    return 0
  fi
  curl --fail --silent --show-error --max-time 60 "$TP_RAW/$name" -o "$dest"
}

# 用选中的模型发一次很小的真实请求。$1 = messages | responses
tp_probe() {
  local status
  step "正在用 $MODEL 测试连接..."
  if [ "$1" = messages ]; then
    status="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$TP_GATEWAY/v1/messages" \
      -H "x-api-key: $API_KEY" -H 'anthropic-version: 2023-06-01' -H 'Content-Type: application/json' \
      -d "{\"model\":\"$MODEL\",\"max_tokens\":16,\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}" \
      --max-time 45 2>/dev/null || echo 000)"
  else
    status="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$TP_GATEWAY/v1/responses" \
      -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
      -d "{\"model\":\"$MODEL\",\"max_output_tokens\":16,\"input\":\"hi\"}" \
      --max-time 45 2>/dev/null || echo 000)"
  fi
  if [ "$status" = 200 ]; then
    green '连接正常'
  else
    yellow "连接测试没通过（HTTP ${status}）"
    yellow '配置已经写好；可以换一个模型重新运行，或到 TokenPlus 看一下余额'
  fi
}

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
