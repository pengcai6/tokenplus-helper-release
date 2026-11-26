# Grok 工具逻辑（拼接在 common.ps1 之后）

$ProviderId = 'tokenplus'

# 去掉本工具写过的模型段（id 以 tokenplus 开头，含桌面版自动生成的子段；旧版 yituo-grok 段一并清掉），
# 把 [models] 的 default 换成新值，最后追加新的模型段。
# 空行先攒着，遇到要保留的内容才补上，反复运行不会越堆越多。
function WriteGrokToml($target, $defaultId, $sections) {
    $parent = Split-Path -Parent $target
    if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    if (Test-Path -LiteralPath $target) {
        Copy-Item -LiteralPath $target -Destination ($target + '.bak-' + $stamp) -Force
        $lines = @((Get-Content -LiteralPath $target -Raw -Encoding UTF8) -split '\r?\n')
    } else {
        $lines = @()
    }

    $out = New-Object System.Collections.Generic.List[string]
    $skip = $false
    $inModels = $false
    $blanks = 0
    $found = $false
    foreach ($line in $lines) {
        if ($line -match '^[ \t]*$') {
            if (-not $skip) { $blanks++ }
            continue
        }
        if ($line -match '^[ \t]*\[') {
            $skip = ($line -match '^[ \t]*\[[ \t]*model[ \t]*\.[ \t]*"?(tokenplus|yituo-grok)')
            $inModels = ($line -match '^[ \t]*\[[ \t]*models[ \t]*\][ \t]*(#.*)?$')
            if ($skip) { continue }
            while ($blanks -gt 0) { $out.Add(''); $blanks-- }
            $out.Add($line)
            if ($inModels) {
                $out.Add('default = "' + $defaultId + '"')
                $found = $true
            }
            continue
        }
        if ($skip) { continue }
        if ($inModels -and $line -match '^[ \t]*default[ \t]*=') { continue }
        while ($blanks -gt 0) { $out.Add(''); $blanks-- }
        $out.Add($line)
    }
    if (-not $found) {
        $out.Add('')
        $out.Add('[models]')
        $out.Add('default = "' + $defaultId + '"')
    }
    $out.Add('')
    foreach ($sectionLine in ($sections -split '\r?\n')) { $out.Add($sectionLine) }

    WriteTextFile $target ([string]::Join($NL, $out) + $NL)
}

TP-Banner 'Grok 一键配置'
TP-CheckGateway
TP-RequireKey
TP-RequireModel
$BaseUrl = $TPGateway + '/v1'
$ModelDefault = $script:TPModel

# ---------- 命令行版 Grok Build ----------
$env:Path = $env:Path + ';' + (Join-Path $env:USERPROFILE '.local\bin') + ';' + (Join-Path $env:USERPROFILE '.grok\bin')
$cliBin = Get-Command grok -ErrorAction SilentlyContinue
if ($cliBin) {
    Ok ('Grok Build 已安装：' + $cliBin.Source)
} elseif ($env:TOKENPLUS_NO_INSTALL) {
    Note '没有检测到 grok 命令，已按要求跳过安装，配置仍会写好'
} else {
    Step '正在用 xAI 官方脚本安装 Grok Build...'
    $installed = $false
    try {
        Invoke-Expression (Invoke-WebRequest -UseBasicParsing -Uri 'https://x.ai/cli/install.ps1' -TimeoutSec 30).Content
        $cliBin = Get-Command grok -ErrorAction SilentlyContinue
        if ($cliBin) { $installed = $true }
    } catch {}
    if ($installed) {
        Ok ('已安装到 ' + $cliBin.Source)
    } else {
        Note 'Grok Build 安装失败（可能需要代理访问 x.ai），配置仍会写好'
        Note '稍后可手动运行：irm https://x.ai/cli/install.ps1 | iex'
    }
}

$cliSections = ''
$appModels = ''
foreach ($name in $script:TPModels) {
    $cliSections += '[model."' + $ProviderId + '/' + $name + '"]' + $NL
    $cliSections += 'model = "' + $name + '"' + $NL
    $cliSections += 'name = "TokenPlus · ' + $name + '"' + $NL
    $cliSections += 'base_url = "' + $BaseUrl + '"' + $NL
    $cliSections += 'api_key = "' + $script:TPApiKey + '"' + $NL
    $cliSections += 'api_backend = "responses"' + $NL
    $cliSections += 'supports_reasoning_effort = true' + $NL + $NL
    if ($appModels) { $appModels += ',' }
    $appModels += '{"id":"' + $name + '","name":"' + $name + '"}'
}

Step '正在写入 Grok Build 配置...'
$grokHome = $env:GROK_HOME
if (-not $grokHome) { $grokHome = Join-Path $env:USERPROFILE '.grok' }
$cliConfig = Join-Path $grokHome 'config.toml'
WriteGrokToml $cliConfig ($ProviderId + '/' + $ModelDefault) $cliSections
Ok ('已写入 ' + $cliConfig)

# ---------- Grok 桌面版 ----------
# 桌面版用 Rust directories 的 data_dir，Windows 上是 %APPDATA%\grokapp\grok-app\data；
# 老版本或手动迁移过的数据可能直接在上一层，已有 agent-home 的那个优先。
$appHome = $env:GROK_APP_HOME
if (-not $appHome) {
    $appRoot = Join-Path $env:APPDATA 'grokapp\grok-app'
    $appHome = Join-Path $appRoot 'data'
    if (-not (Test-Path -LiteralPath (Join-Path $appHome 'agent-home')) -and (Test-Path -LiteralPath (Join-Path $appRoot 'agent-home'))) {
        $appHome = $appRoot
    }
}
$appSection = '[model.' + $ProviderId + ']' + $NL
$appSection += 'model = "' + $ModelDefault + '"' + $NL
$appSection += 'name = "TokenPlus · Grok"' + $NL
$appSection += 'base_url = "' + $BaseUrl + '"' + $NL
$appSection += 'api_key = "' + $script:TPApiKey + '"' + $NL
$appSection += 'api_backend = "responses"' + $NL
$appSection += 'supports_reasoning_effort = true' + $NL
$appSection += 'app_provider_mode = "generic"' + $NL
$appSection += "app_models = '[" + $appModels + "]'" + $NL

Step '正在写入 Grok 桌面版配置...'
$appConfig = Join-Path $appHome 'agent-home\config.toml'
WriteGrokToml $appConfig $ProviderId $appSection
Ok ('已写入 ' + $appConfig)

TP-Probe responses

Write-Host ''
Write-Host '  配置完成，默认模型 ' -NoNewline -ForegroundColor Green
Write-Host $ModelDefault -ForegroundColor Green
Write-Host '  命令行：新开一个终端，敲 grok（/model 可切换其他模型）'
Write-Host '  桌面版：完全退出 Grok 后重新打开，在模型列表里选「TokenPlus · Grok」'
TP-Exit 0
