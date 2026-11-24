#!/usr/bin/env bash
# TokenPlus · DeepSeek Harness 一键配置（macOS）
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/deepseek-setup.sh | bash
#     自动打开 TokenPlus，登录后复制 API Key 粘贴到终端回车即可。
#   curl -fsSL https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/deepseek-setup.sh | bash -s -- sk-你的key 模型名
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
