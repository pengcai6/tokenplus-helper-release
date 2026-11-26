# Claude 桌面版工具逻辑（拼接在 common.ps1 之后）

# 本站在桌面版配置库里的固定编号，重复运行覆盖同一条而不是越堆越多。
# 与 macOS 版共用同一个编号；模型由桌面版从网关自动发现，无需选择。
$ProfileId = '7a1c9f60-5b2e-4c3a-9d18-3f6b0e2a41d7'
$BrandName = 'TokenPlus'

# 桌面版可能是传统安装包，也可能是 Microsoft Store 的 MSIX 安装。
# 不能按进程名判断：命令行版的进程名也叫 claude，用户开着命令行版时会被误判成装了桌面版。
# 也不能只看目录存在：本工具自己会在那个目录下建配置库，判断会变成自我肯定。
# 所以只认三样东西 —— 应用程序本体、卸载登记和 Store 包目录。
function HasClaudeDesktop() {
    foreach ($path in @((Join-Path $env:LOCALAPPDATA 'Programs\Claude\Claude.exe'), (Join-Path $env:LOCALAPPDATA 'AnthropicClaude\Claude.exe'))) {
        if (Test-Path -LiteralPath $path) { return $true }
    }
    foreach ($dir in (DesktopDataRoots)) {
        if (-not (Test-Path -LiteralPath $dir)) { continue }
        $exe = Get-ChildItem -LiteralPath $dir -Filter '*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($exe) { return $true }
    }
    $roots = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    )
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($item in (Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue)) {
            $name = (Get-ItemProperty -LiteralPath $item.PSPath -ErrorAction SilentlyContinue).DisplayName
            if ($name -and $name -like 'Claude*' -and $name -notlike 'Claude Code*') { return $true }
        }
    }
    $packagesDir = Join-Path $env:LOCALAPPDATA 'Packages'
    if (Test-Path -LiteralPath $packagesDir) {
        $package = Get-ChildItem -LiteralPath $packagesDir -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like 'Claude_*' -or $_.Name -like '*Anthropic*' } |
            Select-Object -First 1
        if ($package) { return $true }
    }
    return $false
}

# 官方第三方配置库位置；Store 安装补充包缓存路径。
# https://claude.com/docs/third-party/claude-desktop/configuration
function DesktopDataRoots() {
    $roots = @((Join-Path $env:LOCALAPPDATA 'Claude-3p'))
    $packagesDir = Join-Path $env:LOCALAPPDATA 'Packages'
    if (Test-Path -LiteralPath $packagesDir) {
        $packages = Get-ChildItem -LiteralPath $packagesDir -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like 'Claude_*' -or $_.Name -like '*Anthropic*' }
        foreach ($package in $packages) {
            $roots += Join-Path $package.FullName 'LocalCache\Roaming\Claude-3p'
            $roots += Join-Path $package.FullName 'LocalCache\Local\Claude-3p'
        }
    }
    return @($roots | Select-Object -Unique)
}

function DesktopConfigDirs() {
    return @(DesktopDataRoots | ForEach-Object { Join-Path $_ 'configLibrary' } | Select-Object -Unique)
}

# 配置库只描述网关，deploymentMode 决定桌面版启动时是否进入第三方部署模式。
# 某些版本会把它持久化成 1p；不写这个文件时，应用会回到登录页而不会读取网关配置。
function WriteDesktopMode($root, $stamp) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    $modePath = Join-Path $root 'claude_desktop_config.json'
    if (Test-Path -LiteralPath $modePath) {
        Copy-Item -LiteralPath $modePath -Destination ($modePath + '.bak-' + $stamp) -Force
    }
    $mode = ReadJsonFile $modePath
    if (-not $mode) { $mode = New-Object PSObject }
    SetProperty $mode 'deploymentMode' '3p'
    WriteJsonFile $modePath $mode
}

TP-Banner 'Claude 桌面版一键配置'
TP-CheckGateway

try {
    if (-not (HasClaudeDesktop)) { throw '未检测到 Claude 桌面版，请先安装桌面版后重试' }
    # 在要 key 之前先校验将修改的文件，避免白输一次
    foreach ($root in (DesktopDataRoots)) {
        ReadJsonFile (Join-Path $root 'claude_desktop_config.json') | Out-Null
        ReadJsonFile (Join-Path $root ('configLibrary\' + $ProfileId + '.json')) | Out-Null
        $existingMeta = ReadJsonFile (Join-Path $root 'configLibrary\_meta.json')
        if ($existingMeta -and $existingMeta.PSObject.Properties['entries'] -and -not ($existingMeta.entries -is [array])) {
            throw '桌面配置索引 entries 必须是数组，未覆盖'
        }
    }

    TP-RequireKey

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    Step '正在写入桌面版配置...'
    $gatewayProfile = [ordered]@{
        inferenceProvider = 'gateway'
        inferenceCredentialKind = 'static'
        inferenceGatewayBaseUrl = $TPGateway
        inferenceGatewayApiKey = $script:TPApiKey
    }
    foreach ($dir in (DesktopConfigDirs)) {
        $root = Split-Path -Parent $dir
        WriteDesktopMode $root $stamp
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $profilePath = Join-Path $dir ($ProfileId + '.json')
        $profile = ReadJsonFile $profilePath
        if (-not $profile) { $profile = New-Object PSObject }
        if (Test-Path -LiteralPath $profilePath) { Copy-Item -LiteralPath $profilePath -Destination ($profilePath + '.bak-' + $stamp) -Force }
        foreach ($name in $gatewayProfile.Keys) { SetProperty $profile $name $gatewayProfile[$name] }
        # 移除旧的固定模型列表，让桌面版从网关自动发现可用模型。
        $profile.PSObject.Properties.Remove('inferenceModels')
        WriteJsonFile $profilePath $profile
        $metaPath = Join-Path $dir '_meta.json'
        if (Test-Path $metaPath) {
            Copy-Item -LiteralPath $metaPath -Destination ($metaPath + '.bak-' + $stamp) -Force
        }
        $meta = ReadJsonFile $metaPath
        if (-not $meta) { $meta = New-Object PSObject }
        $entries = @()
        if ($meta.PSObject.Properties['entries']) {
            foreach ($entry in $meta.entries) {
                if ($entry -and $entry.id -ne $ProfileId) { $entries += $entry }
            }
        }
        $entries += [ordered]@{ id = $ProfileId; name = $BrandName }
        SetProperty $meta 'entries' ([array]$entries)
        SetProperty $meta 'appliedId' $ProfileId
        WriteJsonFile $metaPath $meta
    }
    Ok '桌面版配置已写入'

    Write-Host ''
    Write-Host '  配置完成' -ForegroundColor Green
    Write-Host '  完全退出 Claude 桌面版后重新打开；模型在应用里选择，列表来自 TokenPlus 网关'
    TP-Exit 0
} catch {
    if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
    Write-Host ''
    Bad $_.Exception.Message
    Write-Host '  可以重新运行本工具再试一次'
    TP-Exit 1
}
