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
