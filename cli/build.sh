#!/bin/bash
# 把 cli/src 下的公共部分和各工具脚本拼成可独立运行的 <工具>-setup.sh（放在仓库根目录）。
set -euo pipefail
cd "$(dirname "$0")"
RAW='https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main'

build() {
  local name="$1" title="$2" out="../$3" args="$4"
  {
    printf '#!/usr/bin/env bash\n'
    printf '# TokenPlus · %s（macOS）\n#\n' "$title"
    printf '# 用法：\n'
    printf '#   curl -fsSL %s/%s | bash\n' "$RAW" "$3"
    printf '#     自动打开 TokenPlus，登录后复制 API Key 粘贴到终端回车即可。\n'
    printf '#   curl -fsSL %s/%s | bash -s -- %s\n' "$RAW" "$3" "$args"
    printf '#     直接带参数，不再提问。\n#\n'
    printf '# 配置文件写入前都会先备份，只改本工具负责的那几项。\n'
    printf '# 本文件由 cli/build.sh 生成，请修改 cli/src/ 下的源文件。\n\n'
    cat src/common.sh
    printf '\n'
    cat "src/$name.sh"
  } > "$out"
  chmod +x "$out"
  bash -n "$out"
}

build claude 'Claude Code 一键配置' claude-setup.sh 'sk-你的key 模型名'
build claude-desktop 'Claude 桌面版一键配置' claude-desktop-setup.sh 'sk-你的key'
build codex 'Codex 一键配置' codex-setup.sh 'sk-你的key 模型名'
build grok 'Grok 一键配置' grok-setup.sh 'sk-你的key 模型名'
build deepseek 'DeepSeek Harness 一键配置' deepseek-setup.sh 'sk-你的key 模型名'
build workbuddy 'WorkBuddy 一键配置' workbuddy-setup.sh 'sk-你的key'
build setup '多客户端一键配置' setup.sh 'sk-你的key 模型名'
echo "built: $(ls ../*-setup.sh ../setup.sh | wc -l | tr -d ' ') scripts"
