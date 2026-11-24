# JSON 工具只用来合并用户已有的配置文件
detect_json_runner() {
  if command -v python3 >/dev/null 2>&1; then
    printf 'python3'
  elif command -v node >/dev/null 2>&1; then
    printf 'node'
  else
    printf ''
  fi
}

# 把若干「键=值」合并进 JSON 文件的 env 对象，其余内容原样保留
merge_env_into_json() {
  target="$1"
  shift
  case "$JSON_RUNNER" in
    python3)
      python3 -c '
import json, os, sys
path = sys.argv[1]
data = {}
if os.path.exists(path):
    with open(path, "r", encoding="utf-8") as fh:
        text = fh.read().strip()
    if text:
        data = json.loads(text)
if not isinstance(data, dict):
    raise ValueError("配置文件必须是 JSON 对象，未覆盖")
env = data.get("env")
if not isinstance(env, dict):
    env = {}
for pair in sys.argv[2:]:
    key, _, value = pair.partition("=")
    env[key] = value
data["env"] = env
with open(path, "w", encoding="utf-8") as fh:
    json.dump(data, fh, indent=2, ensure_ascii=False)
    fh.write("\n")
' "$target" "$@"
      ;;
    node)
      node -e '
const fs = require("fs")
const path = process.argv[1]
let data = {}
if (fs.existsSync(path)) {
  const text = fs.readFileSync(path, "utf8").trim()
  if (text) data = JSON.parse(text)
}
if (typeof data !== "object" || data === null || Array.isArray(data)) throw new Error("配置文件必须是 JSON 对象，未覆盖")
let env = data.env
if (typeof env !== "object" || env === null || Array.isArray(env)) env = {}
for (const pair of process.argv.slice(2)) {
  const at = pair.indexOf("=")
  env[pair.slice(0, at)] = pair.slice(at + 1)
}
data.env = env
fs.writeFileSync(path, JSON.stringify(data, null, 2) + "\n")
' "$target" "$@"
      ;;
    *)
      return 1
      ;;
  esac
}

tp_banner 'Claude Code 一键配置'

JSON_RUNNER="$(detect_json_runner)"
[ -n "$JSON_RUNNER" ] || { red '需要 Python 3 或 Node.js 保留已有配置，请安装后重试'; exit 1; }

tp_require_key
tp_require_model

CLI_BIN="$(command -v claude 2>/dev/null || true)"
if [ -n "$CLI_BIN" ]; then
  green "命令行版已安装：$CLI_BIN"
else
  yellow '没有检测到 claude 命令，配置仍会写好'
  yellow '安装命令行版：curl -fsSL https://claude.ai/install.sh | bash'
fi

step '正在写入命令行版配置...'
CLAUDE_DIR="$HOME/.claude"
SETTINGS_PATH="$CLAUDE_DIR/settings.json"
mkdir -p "$CLAUDE_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
if [ -f "$SETTINGS_PATH" ]; then
  cp "$SETTINGS_PATH" "$SETTINGS_PATH.bak-$STAMP"
fi

# Claude Code 按 opus / sonnet / haiku 档位要模型，四档都指向选中的同一个模型
merge_env_into_json "$SETTINGS_PATH" \
  "ANTHROPIC_BASE_URL=$TP_GATEWAY" \
  "ANTHROPIC_AUTH_TOKEN=$API_KEY" \
  "ANTHROPIC_MODEL=$MODEL" \
  "ANTHROPIC_DEFAULT_OPUS_MODEL=$MODEL" \
  "ANTHROPIC_DEFAULT_SONNET_MODEL=$MODEL" \
  "ANTHROPIC_DEFAULT_HAIKU_MODEL=$MODEL" \
  "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1" \
  "DISABLE_AUTOUPDATER=1"
chmod 600 "$SETTINGS_PATH"
green "已写入 $SETTINGS_PATH"

tp_probe messages

printf '\n'
printf '\033[32m  配置完成，模型 %s\033[0m\n' "$MODEL"
if [ -n "$CLI_BIN" ]; then
  printf '  命令行：新开一个终端，敲 claude\n'
fi
printf '  opus / sonnet / haiku 三个档位都使用 %s\n' "$MODEL"
