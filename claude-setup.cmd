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
# TokenPlus · Claude Code 一键配置（Windows）
#
# 用法一（无需下载）：
#   powershell -NoProfile -Command "irm https://raw.githubusercontent.com/pengcai6/tokenplus-helper-release/main/claude-setup.ps1 | iex"
# 用法二（双击）：下载 claude-setup.cmd 后双击运行
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
# Claude Code 工具逻辑（拼接在 common.ps1 之后）

TP-Banner 'Claude Code 一键配置'
TP-CheckGateway
TP-RequireKey
TP-RequireModel

try {
    $claudeDir = Join-Path $env:USERPROFILE '.claude'
    if (-not (Test-Path $claudeDir)) { New-Item -ItemType Directory -Path $claudeDir -Force | Out-Null }
    $settingsPath = Join-Path $claudeDir 'settings.json'
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'

    $settings = $null
    if (Test-Path -LiteralPath $settingsPath) {
        $settings = ReadJsonFile $settingsPath
        if (-not $settings) {
            $settings = New-Object PSObject
        } elseif ($settings -isnot [PSCustomObject]) {
            throw 'settings.json 顶层不是 JSON 对象，未覆盖'
        }
        Copy-Item -LiteralPath $settingsPath -Destination ($settingsPath + '.bak-' + $stamp) -Force
    } else {
        $settings = New-Object PSObject
    }

    $envObj = $settings.PSObject.Properties['env']
    if (-not $envObj -or -not $envObj.Value -or $envObj.Value -isnot [PSCustomObject]) {
        SetProperty $settings 'env' (New-Object PSObject)
    }
    $envObj = $settings.env

    SetProperty $envObj 'ANTHROPIC_BASE_URL' $TPGateway
    SetProperty $envObj 'ANTHROPIC_AUTH_TOKEN' $script:TPApiKey
    SetProperty $envObj 'ANTHROPIC_MODEL' $script:TPModel
    SetProperty $envObj 'ANTHROPIC_DEFAULT_OPUS_MODEL' $script:TPModel
    SetProperty $envObj 'ANTHROPIC_DEFAULT_SONNET_MODEL' $script:TPModel
    SetProperty $envObj 'ANTHROPIC_DEFAULT_HAIKU_MODEL' $script:TPModel
    SetProperty $envObj 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC' '1'
    SetProperty $envObj 'DISABLE_AUTOUPDATER' '1'

    WriteJsonFile $settingsPath $settings
    Ok ('已写入 ' + $settingsPath)

    TP-Probe messages

    Write-Host ''
    Write-Host '  配置完成，主模型 ' -NoNewline -ForegroundColor Green
    Write-Host $script:TPModel -ForegroundColor Green
    if (Get-Command claude -ErrorAction SilentlyContinue) {
        Write-Host '  新开一个终端运行 claude 即可使用'
    } else {
        Write-Host '  未检测到 claude 命令；安装 Claude Code 后直接可用，不用再运行本工具'
    }
    TP-Exit 0
} catch {
    if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
    Write-Host ''
    Bad $_.Exception.Message
    Write-Host '  可以重新运行本工具再试一次'
    TP-Exit 1
}

} catch {
    if ($_.Exception.Message -ne $script:TPSentinel) {
        Bad $_.Exception.Message
        $script:TPExitCode = 1
    }
}
if ($script:TPCanExit) { exit $script:TPExitCode }
