tp_banner 'Claude 桌面版一键配置'

command -v python3 >/dev/null 2>&1 || { red '需要 Python 3 合并并保留已有配置，请安装后重试'; exit 1; }
[ -d '/Applications/Claude.app' ] || [ -d "$HOME/Applications/Claude.app" ] || { red '请先安装 Claude 桌面版'; exit 1; }
desktop_root="$HOME/Library/Application Support/Claude-3p"

# 在要 key 之前先校验将修改的文件，避免白输一次
python3 - "$desktop_root" <<'PY' || { red '现有桌面配置无法读取，未做修改'; exit 1; }
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
for path in [root / 'claude_desktop_config.json', root / 'configLibrary/_meta.json', root / 'configLibrary/7a1c9f60-5b2e-4c3a-9d18-3f6b0e2a41d7.json']:
    if path.exists():
        data = json.loads(path.read_text(encoding='utf-8'))
        if not isinstance(data, dict):
            raise ValueError(f'配置必须是 JSON 对象，未覆盖：{path}')
        if path.name == '_meta.json' and 'entries' in data and not isinstance(data['entries'], list):
            raise ValueError(f'桌面配置索引 entries 必须是数组，未覆盖：{path}')
PY

tp_require_key

step '正在写入桌面版配置...'
TP_KEY="$API_KEY" TP_BASE="$TP_GATEWAY" python3 -c '
import datetime, json, os, pathlib, shutil, sys, tempfile
root = pathlib.Path(sys.argv[1])
config = {"api_key": os.environ["TP_KEY"], "gateway_base_url": os.environ["TP_BASE"]}
profile_id = "7a1c9f60-5b2e-4c3a-9d18-3f6b0e2a41d7"
library = root / "configLibrary"
def read(path):
    if not path.exists(): return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict): raise ValueError(f"配置必须是 JSON 对象：{path}")
    return data
profile_path = library / (profile_id + ".json"); meta_path = library / "_meta.json"; mode_path = root / "claude_desktop_config.json"
profile = read(profile_path); meta = read(meta_path); mode = read(mode_path)
profile.update(inferenceProvider="gateway", inferenceCredentialKind="static", inferenceGatewayBaseUrl=config["gateway_base_url"], inferenceGatewayApiKey=config["api_key"])
# Remove the legacy fixed list so the desktop discovers models from the gateway.
profile.pop("inferenceModels", None)
entries = meta.get("entries", [])
if not isinstance(entries, list): raise ValueError("桌面配置索引 entries 必须是数组，未覆盖")
meta["entries"] = [entry for entry in entries if not (isinstance(entry, dict) and entry.get("id") == profile_id)] + [{"id":profile_id, "name":"TokenPlus"}]
meta["appliedId"] = profile_id; mode["deploymentMode"] = "3p"
library.mkdir(parents=True, exist_ok=True, mode=0o700)
stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
for path, data in [(profile_path, profile), (mode_path, mode), (meta_path, meta)]:
    if path.exists():
        backup = pathlib.Path(str(path) + ".bak-" + stamp); shutil.copy2(path, backup); backup.chmod(0o600)
    descriptor, temporary = tempfile.mkstemp(dir=path.parent, prefix=".setup-")
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle: json.dump(data, handle, ensure_ascii=False, indent=2); handle.write("\n")
        os.replace(temporary, path); path.chmod(0o600)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)
' "$desktop_root"
green "已写入 $desktop_root"

printf '\n'
printf '\033[32m  配置完成\033[0m\n'
printf '  完全退出 Claude 桌面版后重新打开；模型在应用里选择，列表来自 TokenPlus 网关\n'
