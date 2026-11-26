#!/bin/bash
# 把 cli/src/win 下的公共部分和各工具脚本拼成可独立运行的 <工具>-setup.ps1（放在仓库根目录）。
# 每个文件同时是 cmd/PowerShell 双格式脚本：PowerShell 把开头几行当块注释跳过，
# cmd.exe 双击运行时由批处理头调起 PowerShell 执行正文。
# 产物行尾统一转成 CRLF：cmd.exe 解析批处理头需要 CRLF，PowerShell 两种都认。
set -euo pipefail
cd "$(dirname "$0")"
RAW='https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main'

header() {
  cat <<'EOF'
<# :
@echo off
chcp 65001 >nul
set "TP_SELF=%~f0"
set "TP_PS=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if exist "%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe" set "TP_PS=%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
"%TP_PS%" -NoProfile -ExecutionPolicy Bypass -Command "& ([scriptblock]::Create((Get-Content -LiteralPath $env:TP_SELF -Raw -Encoding UTF8)))"
set "TP_EXIT_CODE=%errorlevel%"
set "TP_SELF="
set "TP_PS="
echo.
pause
exit /b %TP_EXIT_CODE%
: #>
EOF
}

build() {
  local name="$1" title="$2" out="../$3"
  {
    header
    printf 'param(\n  [string]$ApiKey = $env:TOKENPLUS_API_KEY,\n  [string]$Model = $env:TOKENPLUS_MODEL\n)\n'
    printf '# TokenPlus · %s（Windows）\n#\n' "$title"
    printf '# 用法一（无需下载）：\n'
    printf '#   powershell -NoProfile -Command "irm %s/%s | iex"\n' "$RAW" "$3"
    printf '# 用法二（双击）：下载 %s-setup.cmd 后双击运行\n' "${3%-setup.ps1}"
    printf '# 免提问：先设置环境变量 TOKENPLUS_API_KEY（和 TOKENPLUS_MODEL）再运行\n#\n'
    printf '# 配置文件写入前都会先备份，只改本工具负责的那几项。\n'
    printf '# 本文件由 cli/build-win.sh 生成，请修改 cli/src/win/ 下的源文件。\n\n'
    printf '$script:TPApiKey = $ApiKey\n$script:TPModel = $Model\n$script:TPModels = @()\n\n'
    cat src/win/common.ps1
    printf '\ntry {\n'
    cat "src/win/$name.ps1"
    printf '\n} catch {\n'
    printf '    if ($_.Exception.Message -ne $script:TPSentinel) {\n'
    printf "        Bad \$_.Exception.Message\n"
    printf '        $script:TPExitCode = 1\n'
    printf '    }\n'
    printf '}\n'
    printf 'if ($script:TPCanExit) { exit $script:TPExitCode }\n'
  } | perl -pe 's/\r?\n$/\r\n/' > "$out"
  # ps1 加 BOM：PowerShell 5.1 按 ANSI 读无 BOM 的 -File 脚本，中文会乱码。
  # cmd 不加 BOM：批处理段是纯 ASCII，且 cmd 对 BOM 的容忍度没有保证；
  # 正文由批处理头显式按 UTF8 读入，不受影响。
  printf '\xef\xbb\xbf' | cat - "$out" > "$out.tmp" && mv "$out.tmp" "$out"
  cp "$out" "$out.nobom" && perl -i -pe 's/^\xef\xbb\xbf// if $. == 1' "$out.nobom"
  mv "$out.nobom" "../${3%.ps1}.cmd"
}

build claude 'Claude Code 一键配置' claude-setup.ps1
build claude-desktop 'Claude 桌面版一键配置' claude-desktop-setup.ps1
build codex 'Codex 一键配置' codex-setup.ps1
build grok 'Grok 一键配置' grok-setup.ps1
build deepseek 'DeepSeek Harness 一键配置' deepseek-setup.ps1
build workbuddy 'WorkBuddy 一键配置' workbuddy-setup.ps1
build setup '多客户端一键配置' setup.ps1
echo "built: $(ls ../*-setup.ps1 ../*-setup.cmd ../setup.ps1 ../setup.cmd 2>/dev/null | wc -l | tr -d ' ') files"
