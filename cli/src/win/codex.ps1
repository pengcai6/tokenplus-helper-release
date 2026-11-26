# Codex 工具逻辑（拼接在 common.ps1 之后）

$BeginBase = '# >>> tokenplus codex setup (base) >>>'
$EndBase = '# <<< tokenplus codex setup (base) <<<'
$BeginProvider = '# >>> tokenplus codex setup (provider) >>>'
$EndProvider = '# <<< tokenplus codex setup (provider) <<<'
$ProviderId = 'tokenplus'
$ProviderName = 'TokenPlus'

# 去掉一对标记之间的内容，重复运行时覆盖上一次写入的配置
function StripMarked($lines, $begin, $end) {
    $out = @()
    $inside = $false
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ($trimmed -eq $begin) { $inside = $true; continue }
        if ($trimmed -eq $end) { $inside = $false; continue }
        if (-not $inside) { $out += $line }
    }
    return ,$out
}

# 去掉同名的表定义，避免和本工具写入的表重复
function StripTable($lines, $header) {
    $out = @()
    $inside = $false
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ($trimmed -eq $header) { $inside = $true; continue }
        if ($inside -and $trimmed.StartsWith('[')) { $inside = $false }
        if (-not $inside) { $out += $line }
    }
    return ,$out
}

# 顶层同名键会和本工具写入的键冲突，注释掉
function CommentTopLevelKey($lines, $key) {
    $out = @()
    $topLevel = $true
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ($trimmed.StartsWith('[')) { $topLevel = $false }
        if ($topLevel -and $trimmed -match ('^' + [regex]::Escape($key) + '\s*=')) {
            $out += ('# ' + $line)
            continue
        }
        $out += $line
    }
    return ,$out
}

function TrimBlankEdges($lines) {
    $list = New-Object System.Collections.ArrayList
    foreach ($line in $lines) { [void]$list.Add($line) }
    while ($list.Count -gt 0 -and $list[0].Trim() -eq '') { $list.RemoveAt(0) }
    while ($list.Count -gt 0 -and $list[$list.Count - 1].Trim() -eq '') { $list.RemoveAt($list.Count - 1) }
    return ,$list.ToArray()
}

function HasCodexDesktop() {
    $roots = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    )
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($item in (Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue)) {
            $name = (Get-ItemProperty -LiteralPath $item.PSPath -ErrorAction SilentlyContinue).DisplayName
            if ($name -and $name -like '*Codex*') { return $true }
        }
    }
    try {
        if (Get-Command Get-AppxPackage -ErrorAction SilentlyContinue) {
            if (Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue) { return $true }
        }
    } catch {}
    return $false
}

function HasCodexCli() {
    return [bool](Get-Command codex -ErrorAction SilentlyContinue)
}

function CodexPackage() {
    try {
        if (-not (Get-Command Get-AppxPackage -ErrorAction SilentlyContinue)) { return $null }
        return (Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue | Select-Object -First 1)
    } catch {
        if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
        return $null
    }
}

# 从 MSIX 清单读取主应用，兼容 Store 更新、不同架构和官方离线包。
function CodexAppEntry($package) {
    try {
        $apps = @((Get-AppxPackageManifest -Package $package).Package.Applications.Application)
        $app = $apps | Where-Object { $_.Id -eq 'App' } | Select-Object -First 1
        if (-not $app) { $app = $apps | Where-Object { $_.Executable -like '*ChatGPT.exe' } | Select-Object -First 1 }
        if (-not $app) { $app = $apps | Select-Object -First 1 }
        return $app
    } catch {
        if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
        return $null
    }
}

# Codex 同时只允许一个实例，不先退干净的话带参数启动会被直接忽略。
function StopCodexApp() {
    $package = CodexPackage
    if (-not $package -or -not $package.InstallLocation) { return }
    $root = $package.InstallLocation
    $app = CodexAppEntry $package
    $processName = if ($app -and $app.Executable) { [System.IO.Path]::GetFileNameWithoutExtension([string]$app.Executable) } else { 'ChatGPT' }
    $deadline = (Get-Date).AddSeconds(8)
    do {
        $processes = @()
        foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
            try {
                if ($process.Path -and $process.Path.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $processes += $process
                }
            } catch {}
        }
        if ($processes.Count -eq 0) { return }
        foreach ($process in $processes) {
            try {
                if ($process.MainWindowHandle -ne 0) {
                    [void]$process.CloseMainWindow()
                } else {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                }
            } catch { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    $stillRunning = $false
    foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
        try { if ($process.Path -and $process.Path.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) { $stillRunning = $true } } catch {}
    }
    if ($stillRunning) { throw 'Codex 进程未能在 8 秒内退出，请先手动退出 Codex 后重试' }
}

TP-Banner 'Codex 一键配置'
TP-CheckGateway
TP-RequireKey
TP-RequireModel
$BaseUrl = $TPGateway + '/v1'

try {
    # 与 Codex 使用同一个配置目录，兼容 CC Switch 和自定义 CODEX_HOME。
    $codexHome = $env:CODEX_HOME
    if (-not $codexHome) { $codexHome = Join-Path $env:USERPROFILE '.codex' }
    if (-not (Test-Path $codexHome)) { New-Item -ItemType Directory -Path $codexHome -Force | Out-Null }
    $configPath = Join-Path $codexHome 'config.toml'
    $authPath = Join-Path $codexHome 'auth.json'
    $modelCachePath = Join-Path $codexHome 'models_cache.json'
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    while ((Test-Path -LiteralPath ($configPath + '.bak-' + $stamp)) -or (Test-Path -LiteralPath ($modelCachePath + '.bak-' + $stamp))) {
        $stamp += '-1'
    }

    # 配置和模型目录只在启动时加载，先退出桌面客户端，避免旧进程回写缓存。
    # 退不掉不中止整个配置：config.toml 照写不误，用户自己重启 Codex 就会生效。
    if (HasCodexDesktop) {
        Step '正在关闭 Codex 以刷新模型配置...'
        try {
            StopCodexApp
        } catch {
            if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
            Note ($_.Exception.Message + '；配置仍会写入，请稍后手动完全退出并重开 Codex')
        }
    }

    Step '正在写入 Codex 配置...'
    $existing = ''
    if (Test-Path $configPath) {
        Copy-Item -LiteralPath $configPath -Destination ($configPath + '.bak-' + $stamp) -Force
        $existing = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8
    }
    $lines = @($existing -split '\r?\n')
    $lines = StripMarked $lines $BeginBase $EndBase
    $lines = StripMarked $lines $BeginProvider $EndProvider
    # 兼容清理：友商工具（易拓/支书）写过的标记段一并去掉，避免两个供应商并存
    $lines = StripMarked $lines '# >>> yituohub codex setup (base) >>>' '# <<< yituohub codex setup (base) <<<'
    $lines = StripMarked $lines '# >>> yituohub codex setup (provider) >>>' '# <<< yituohub codex setup (provider) <<<'
    $lines = StripMarked $lines '# >>> zhishuhub codex setup (base) >>>' '# <<< zhishuhub codex setup (base) <<<'
    $lines = StripMarked $lines '# >>> zhishuhub codex setup (provider) >>>' '# <<< zhishuhub codex setup (provider) <<<'
    $lines = StripTable $lines ('[model_providers.' + $ProviderId + ']')
    $lines = StripTable $lines '[model_providers.yituohub]'
    $lines = StripTable $lines '[model_providers.zhishuhub]'
    $lines = CommentTopLevelKey $lines 'model'
    $lines = CommentTopLevelKey $lines 'model_provider'
    $lines = CommentTopLevelKey $lines 'model_reasoning_effort'
    # CC Switch 的模型目录会让下拉菜单继续显示旧供应商的模型。
    $lines = CommentTopLevelKey $lines 'model_catalog_json'
    $lines = CommentTopLevelKey $lines 'profile'
    $lines = CommentTopLevelKey $lines 'approval_policy'
    $lines = CommentTopLevelKey $lines 'sandbox_mode'
    $lines = TrimBlankEdges $lines

    $head = @()
    $head += $BeginBase
    $head += ('model = "' + $script:TPModel + '"')
    $head += ('model_provider = "' + $ProviderId + '"')
    $head += 'model_reasoning_effort = "high"'
    $head += 'approval_policy = "never"'
    $head += 'sandbox_mode = "danger-full-access"'
    $head += $EndBase

    $tail = @()
    $tail += $BeginProvider
    $tail += ('[model_providers.' + $ProviderId + ']')
    $tail += ('name = "' + $ProviderName + '"')
    $tail += ('base_url = "' + $BaseUrl + '"')
    $tail += 'wire_api = "responses"'
    $tail += ('experimental_bearer_token = "' + $script:TPApiKey + '"')
    $tail += 'requires_openai_auth = false'
    $tail += 'http_headers = { "x-openai-actor-authorization" = "tokenplus" }'
    $tail += $EndProvider

    $merged = @()
    $merged += $head
    if ($lines.Count -gt 0) {
        $merged += ''
        $merged += $lines
    }
    $merged += ''
    $merged += $tail
    WriteTextFile $configPath ([string]::Join($NL, $merged) + $NL)
    Ok ('已写入 ' + $configPath)
    Ok '已设置完全访问权限，重启 Codex 后生效'

    if (Test-Path -LiteralPath $modelCachePath) {
        Move-Item -LiteralPath $modelCachePath -Destination ($modelCachePath + '.bak-' + $stamp)
        Ok '旧模型列表已备份，重启后会重新加载'
    }

    # 密钥跟着 provider 一起写在 config.toml 里，不进 auth.json：
    # 写进 auth.json 会把 Codex 的身份切成 API Key，ChatGPT 登录和挂在那个
    # 账号下的会话记录就都看不到了。顺手清掉其他工具早先写进去的那一把
    # （值对得上才动，不碰用户自己的密钥）。
    if (Test-Path $authPath) {
        try {
            $rawAuth = Get-Content -LiteralPath $authPath -Raw -Encoding UTF8
            if ($rawAuth -and $rawAuth.Trim()) {
                $parsed = $rawAuth | ConvertFrom-Json
                $existing = $parsed.PSObject.Properties['OPENAI_API_KEY']
                if ($existing -and $existing.Value -eq $script:TPApiKey) {
                    Copy-Item -LiteralPath $authPath -Destination ($authPath + '.bak-' + $stamp) -Force
                    $auth = @{}
                    foreach ($property in $parsed.PSObject.Properties) {
                        if ($property.Name -ne 'OPENAI_API_KEY') { $auth[$property.Name] = $property.Value }
                    }
                    WriteTextFile $authPath (($auth | ConvertTo-Json -Depth 10) + $NL)
                    Ok '已恢复 Codex 原来的登录身份'
                }
            }
        } catch {
            if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
            Note '读不出 auth.json，已跳过'
        }
    }

    # 密钥现在只存在于 config.toml，清掉早先版本留下的同名环境变量
    foreach ($legacyEnvKey in @('TOKENPLUS_API_KEY', 'YITUOHUB_API_KEY', 'ZHISHUHUB_API_KEY')) {
        if ([Environment]::GetEnvironmentVariable($legacyEnvKey, 'User') -eq $script:TPApiKey) {
            [Environment]::SetEnvironmentVariable($legacyEnvKey, $null, 'User')
        }
    }

    TP-Probe responses

    Write-Host ''
    Write-Host '  配置完成，主模型 ' -NoNewline -ForegroundColor Green
    Write-Host $script:TPModel -ForegroundColor Green
    if (HasCodexDesktop) {
        Write-Host '  桌面客户端：重启 Codex 后即可开始使用'
    }
    if (HasCodexCli) {
        Write-Host '  命令行：直接运行 codex'
    }
    if (-not (HasCodexDesktop) -and -not (HasCodexCli)) {
        Write-Host '  装好 Codex（桌面客户端或命令行版）后直接可用，不用再运行本工具'
    }
    TP-Exit 0
} catch {
    if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
    Write-Host ''
    Bad $_.Exception.Message
    Write-Host '  可以重新运行本工具再试一次'
    TP-Exit 1
}
