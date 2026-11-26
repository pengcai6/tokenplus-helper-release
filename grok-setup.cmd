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
param(
  [string]$ApiKey = $env:TOKENPLUS_API_KEY,
  [string]$Model = $env:TOKENPLUS_MODEL
)
# TokenPlus · Grok 一键配置（Windows）
#
# 用法一（无需下载）：
#   powershell -NoProfile -Command "irm https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/grok-setup.ps1 | iex"
# 用法二（双击）：下载 grok-setup.cmd 后双击运行
# 免提问：先设置环境变量 TOKENPLUS_API_KEY（和 TOKENPLUS_MODEL）再运行
#
# 配置文件写入前都会先备份，只改本工具负责的那几项。
# 本文件由 cli/build-win.sh 生成，请修改 cli/src/win/ 下的源文件。

$script:TPApiKey = $ApiKey
$script:TPModel = $Model
$script:TPModels = @()

# TokenPlus 公共部分：打开网站、粘贴 key、拉取模型、终端选模型。
# 本文件不独立运行，由 cli/build-win.sh 拼进各工具的 -setup.ps1。

$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {}

$TPSite = 'https://www.tokenplus.cn/'
$TPGateway = $env:TOKENPLUS_GATEWAY
if (-not $TPGateway) { $TPGateway = 'https://api.tokenplus.cn' }
$TPGateway = $TPGateway.TrimEnd('/')
$TPRaw = $env:TOKENPLUS_RAW
if (-not $TPRaw) { $TPRaw = 'https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main' }
$NL = [string][char]13 + [string][char]10

# 运行方式决定出错时能不能用 exit：
# irm ... | iex 是在用户自己的终端里执行，exit 会把整个窗口关掉；
# 只有作为文件运行（-File / 双击 .cmd 的子进程）才能安全 exit。
$script:TPExitCode = 0
$script:TPSentinel = 'tokenplus-done-79e4d4f2'
$script:TPCanExit = ([bool]$MyInvocation.MyCommand.Path) -or ([bool]$env:TP_SELF)

function TP-Exit([int]$code) {
    $script:TPExitCode = $code
    if ($script:TPCanExit) { exit $code }
    # irm|iex 模式：抛哨兵异常，由脚本末尾的统一出口接住，不影响用户的终端会话
    throw $script:TPSentinel
}

function Step($text) { Write-Host ('  ' + $text) }
function Ok($text) { Write-Host ('  [完成] ' + $text) -ForegroundColor Green }
function Note($text) { Write-Host ('  [注意] ' + $text) -ForegroundColor Yellow }
function Bad($text) { Write-Host ('  [失败] ' + $text) -ForegroundColor Red }

function WriteTextFile($path, $text) {
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($path, $text, $encoding)
}

# 深度给足，默认只有 2 层，用户配置里的权限和钩子会被压成字符串
function WriteJsonFile($path, $value) {
    WriteTextFile $path (($value | ConvertTo-Json -Depth 100) + $NL)
}

function ReadJsonFile($path) {
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    try {
        $raw = Get-Content -LiteralPath $path -Raw -Encoding UTF8
        if (-not $raw -or -not $raw.Trim()) { return $null }
        return ($raw | ConvertFrom-Json)
    } catch {
        throw ('配置文件不是有效 JSON，未覆盖：' + $path)
    }
}

# 往对象上写属性：已有就改，没有就加
function SetProperty($target, $name, $value) {
    if ($target.PSObject.Properties[$name]) {
        $target.PSObject.Properties[$name].Value = $value
    } else {
        Add-Member -InputObject $target -NotePropertyName $name -NotePropertyValue $value
    }
}

function TP-Banner($title) {
    Write-Host ''
    Write-Host ('  TokenPlus · ' + $title) -ForegroundColor Cyan
    Write-Host '  ---------------------------------------------'
}

function TP-KeyValid($key) {
    return ($key -match '^sk-[A-Za-z0-9_-]+$')
}

# 网关地址只允许 https（本机测试地址除外）
function TP-CheckGateway() {
    if ($TPGateway -match '^https://') { return }
    if ($TPGateway -match '^http://(localhost|127\.0\.0\.1)(:\d+)?$') { return }
    Bad '网关地址必须使用 HTTPS'
    TP-Exit 1
}

# 用 key 读取模型列表。返回 @{ Status = 'ok'|'auth'|'error'; Models = @() }
function TP-FetchModels([string]$Key) {
    try {
        $resp = Invoke-RestMethod -Uri ($TPGateway + '/v1/models') -Headers @{ Authorization = ('Bearer ' + $Key) } -TimeoutSec 25 -ErrorAction Stop
        $ids = New-Object System.Collections.Generic.List[string]
        foreach ($m in @($resp.data)) {
            $id = [string]$m.id
            if ($id -match '^[A-Za-z0-9._:/-]+$' -and -not $ids.Contains($id)) { $ids.Add($id) }
        }
        if ($ids.Count -eq 0) { return @{ Status = 'error'; Models = @() } }
        return @{ Status = 'ok'; Models = $ids.ToArray() }
    } catch {
        $code = 0
        try { $code = [int]$_.Exception.Response.StatusCode } catch {}
        if ($code -eq 401 -or $code -eq 403) { return @{ Status = 'auth'; Models = @() } }
        return @{ Status = 'error'; Models = @() }
    }
}

function TP-CanInteract() {
    if (-not [Environment]::UserInteractive) { return $false }
    try { $null = $Host.UI.RawUI; return $true } catch { return $false }
}

# 没有带 key：打开 TokenPlus，让用户登录后复制 key 粘贴回来
function TP-RequireKey() {
    if ($script:TPApiKey) {
        if (-not (TP-KeyValid $script:TPApiKey)) { Bad 'API Key 格式无效，应以 sk- 开头'; TP-Exit 1 }
        if (-not $script:TPModels -or $script:TPModels.Count -eq 0) {
            Step '正在验证 API Key...'
            $r = TP-FetchModels $script:TPApiKey
            if ($r.Status -eq 'auth') { Bad 'API Key 无效、已停用或已过期'; TP-Exit 1 }
            if ($r.Status -ne 'ok') { Bad '连不上 TokenPlus 网关，请检查网络后重试'; TP-Exit 1 }
            $script:TPModels = $r.Models
        }
        return
    }
    if (-not (TP-CanInteract)) {
        Bad '当前没有可交互的终端'
        Step '请设置环境变量 TOKENPLUS_API_KEY 后重试'
        TP-Exit 1
    }
    Step '正在打开 TokenPlus：登录后创建或复制一个 API Key'
    if (-not $env:TOKENPLUS_NO_OPEN) {
        try { Start-Process $TPSite } catch { Step ('浏览器没有自动打开，请手动访问 ' + $TPSite) }
    }
    $tries = 0
    while ($tries -lt 3) {
        $tries++
        $secure = Read-Host '  › 粘贴 API Key 后回车（输入显示为 *）' -AsSecureString
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) } finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
        $plain = ($plain -replace '\s', '')
        if (-not (TP-KeyValid $plain)) {
            Note 'API Key 格式无效，应以 sk- 开头，请重新粘贴'
            continue
        }
        Step '正在验证 API Key...'
        $r = TP-FetchModels $plain
        if ($r.Status -eq 'ok') {
            $script:TPApiKey = $plain
            $script:TPModels = $r.Models
            return
        }
        if ($r.Status -eq 'auth') {
            Note 'API Key 无效、已停用或已过期，请重新粘贴'
        } else {
            Bad '连不上 TokenPlus 网关，请检查网络后重试'
            TP-Exit 1
        }
    }
    Bad '多次输入无效，已退出'
    TP-Exit 1
}

# 终端里选一个模型：↑↓ 移动，直接打字筛选，回车确认。用 Console API 重绘，不依赖 ANSI。
function TP-PickModel() {
    $all = @($script:TPModels)
    $query = ''
    $cursor = 0
    $top = 0
    $rows = 8
    $drawn = 0
    $width = 70
    try { $width = [Math]::Max(30, [Console]::WindowWidth - 2) } catch {}
    $prevVisible = $true
    try { $prevVisible = [Console]::CursorVisible; [Console]::CursorVisible = $false } catch {}

    $selected = $null
    try {
        while ($true) {
            $shown = @($all | Where-Object { $_.ToLower().Contains($query.ToLower()) })
            $n = $shown.Count
            if ($cursor -ge $n) { $cursor = [Math]::Max(0, $n - 1) }
            if ($cursor -lt $top) { $top = $cursor }
            if ($cursor -ge ($top + $rows)) { $top = $cursor - $rows + 1 }

            if ($drawn -gt 0) {
                try { [Console]::SetCursorPosition(0, [Math]::Max(0, [Console]::CursorTop - $drawn)) } catch {}
            }
            $lines = New-Object System.Collections.Generic.List[string]
            $lines.Add('  ? 选择模型 (↑↓ 移动 · 打字筛选 · 回车确认)')
            $lines.Add('    筛选：' + $query + '▏')
            if ($n -eq 0) { $lines.Add('    没有匹配的模型') }
            for ($i = $top; $i -lt $n -and $i -lt ($top + $rows); $i++) {
                $mark = '    '
                if ($i -eq $cursor) { $mark = '  ❯ ' }
                $lines.Add($mark + $shown[$i])
            }
            $lines.Add('    ' + $n + ' / ' + $all.Count + ' 个模型')
            foreach ($line in $lines) {
                $out = $line
                if ($out.Length -gt $width) { $out = $out.Substring(0, $width) }
                Write-Host ($out.PadRight($width))
            }
            $drawn = $lines.Count

            $key = [Console]::ReadKey($true)
            switch ($key.Key) {
                'Enter' {
                    if ($n -gt 0) { $selected = $shown[$cursor] }
                    break
                }
                'UpArrow' { if ($cursor -gt 0) { $cursor-- } }
                'DownArrow' { if ($cursor -lt ($n - 1)) { $cursor++ } }
                'Backspace' {
                    if ($query.Length -gt 0) { $query = $query.Substring(0, $query.Length - 1) }
                    $cursor = 0; $top = 0
                }
                'Escape' { $query = ''; $cursor = 0; $top = 0 }
                default {
                    $ch = $key.KeyChar
                    if ($ch -and -not [char]::IsControl($ch)) {
                        $query += $ch
                        $cursor = 0; $top = 0
                    }
                }
            }
            if ($selected) { break }
        }
    } finally {
        # 清掉菜单，把光标还给用户
        if ($drawn -gt 0) {
            try {
                [Console]::SetCursorPosition(0, [Math]::Max(0, [Console]::CursorTop - $drawn))
                $blank = ' '.PadRight($width)
                for ($i = 0; $i -lt $drawn; $i++) { Write-Host $blank }
                [Console]::SetCursorPosition(0, [Math]::Max(0, [Console]::CursorTop - $drawn))
            } catch {}
        }
        try { [Console]::CursorVisible = $prevVisible } catch {}
    }
    $script:TPModel = $selected
}

function TP-RequireModel() {
    if ($script:TPModel) {
        if ($script:TPModel -notmatch '^[A-Za-z0-9._:/-]+$') { Bad '模型名包含无效字符'; TP-Exit 1 }
        if (-not (@($script:TPModels) -contains $script:TPModel)) {
            Bad ('模型 ' + $script:TPModel + ' 不在这个 API Key 可用的列表里')
            TP-Exit 1
        }
        return
    }
    if (-not (TP-CanInteract)) {
        Bad '当前没有可交互的终端，无法选择模型'
        Step '请设置环境变量 TOKENPLUS_MODEL 后重试'
        TP-Exit 1
    }
    TP-PickModel
    if (-not $script:TPModel) { Bad '没有选择模型'; TP-Exit 1 }
    Ok ('已选择模型 ' + $script:TPModel)
}

# 取随脚本一起发布的辅助文件：本地有就用本地的，否则从仓库下载
function TP-FetchAsset($name, $dest) {
    if ($env:TOKENPLUS_ASSET_DIR -and (Test-Path -LiteralPath (Join-Path $env:TOKENPLUS_ASSET_DIR $name))) {
        Copy-Item -LiteralPath (Join-Path $env:TOKENPLUS_ASSET_DIR $name) -Destination $dest -Force
        return
    }
    Invoke-WebRequest -UseBasicParsing -Uri ($TPRaw + '/' + $name) -OutFile $dest -TimeoutSec 60
    if (-not (Test-Path -LiteralPath $dest) -or (Get-Item -LiteralPath $dest).Length -eq 0) {
        throw '辅助文件下载失败或为空'
    }
    # PowerShell 5.1 按 ANSI 读无 BOM 的 -File 脚本，中文会乱码；ps1 一律补上 BOM
    if ($name -like '*.ps1') {
        $bytes = [System.IO.File]::ReadAllBytes($dest)
        if ($bytes.Length -ge 3 -and -not ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)) {
            $bom = [byte[]]@(0xEF, 0xBB, 0xBF)
            [System.IO.File]::WriteAllBytes($dest, $bom + $bytes)
        }
    }
}

# 用选中的模型发一次很小的真实请求。-Kind messages | responses
function TP-Probe([string]$Kind) {
    Step ('正在用 ' + $script:TPModel + ' 测试连接...')
    $ok = $false
    try {
        if ($Kind -eq 'messages') {
            $body = @{
                model = $script:TPModel
                max_tokens = 16
                messages = @(@{ role = 'user'; content = 'hi' })
            } | ConvertTo-Json -Depth 10 -Compress
            Invoke-RestMethod -Uri ($TPGateway + '/v1/messages') -Method Post `
                -Headers @{ 'x-api-key' = $script:TPApiKey; 'anthropic-version' = '2023-06-01' } `
                -ContentType 'application/json' -Body $body -TimeoutSec 45 | Out-Null
        } else {
            $body = @{ model = $script:TPModel; max_output_tokens = 16; input = 'hi' } | ConvertTo-Json -Compress
            Invoke-RestMethod -Uri ($TPGateway + '/v1/responses') -Method Post `
                -Headers @{ Authorization = ('Bearer ' + $script:TPApiKey) } `
                -ContentType 'application/json' -Body $body -TimeoutSec 45 | Out-Null
        }
        $ok = $true
    } catch {}
    if ($ok) {
        Ok '连接正常'
    } else {
        Note '连接测试没通过'
        Note '配置已经写好；可以换一个模型重新运行，或到 TokenPlus 看一下余额'
    }
}

try {
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

} catch {
    if ($_.Exception.Message -ne $script:TPSentinel) {
        Bad $_.Exception.Message
        $script:TPExitCode = 1
    }
}
if ($script:TPCanExit) { exit $script:TPExitCode }
