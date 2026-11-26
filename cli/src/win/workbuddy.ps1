# WorkBuddy 工具逻辑（拼接在 common.ps1 之后）

TP-Banner 'WorkBuddy 一键配置'
TP-CheckGateway
TP-RequireKey

$tmp = $null
$previousRunAsNode = $env:ELECTRON_RUN_AS_NODE
try {
    # 使用已安装的 Node.js（20+）或 WorkBuddy 自带的运行时
    $node = Get-Command node.exe -ErrorAction SilentlyContinue
    $runtime = $null
    $electronRuntime = $false
    if ($node) {
        & $node.Source -e 'if (Number(process.versions.node.split(".")[0]) < 20) process.exit(1)' | Out-Null
        if ($LASTEXITCODE -eq 0) { $runtime = $node.Source }
    }
    if (-not $runtime) {
        $roots = @(
            (Join-Path $env:LOCALAPPDATA 'Programs\WorkBuddy'),
            (Join-Path $env:LOCALAPPDATA 'WorkBuddy'),
            (Join-Path $env:LOCALAPPDATA 'Programs\workbuddy'),
            (Join-Path $env:LOCALAPPDATA 'Programs\WorkBuddy AI')
        )
        if ($env:ProgramFiles) {
            $roots += (Join-Path $env:ProgramFiles 'WorkBuddy')
            $roots += (Join-Path $env:ProgramFiles 'WorkBuddy AI')
        }
        foreach ($root in $roots) {
            $exe = Join-Path $root 'WorkBuddy.exe'
            if (Test-Path -LiteralPath $exe) { $runtime = $exe; $electronRuntime = $true; break }
        }
    }
    if (-not $runtime) { throw '请先安装 WorkBuddy，或安装 Node.js 20 及以上版本后重试' }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('tokenplus-workbuddy-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    $script = Join-Path $tmp 'workbuddy-config.mjs'
    TP-FetchAsset 'workbuddy-config.mjs' $script

    Step '正在把 TokenPlus 的模型写入 WorkBuddy...'
    # key 走环境变量，不出现在进程参数里
    $env:TOKENPLUS_API_KEY = $script:TPApiKey
    $env:TOKENPLUS_MODELS = [string]::Join(' ', $script:TPModels)
    $env:TOKENPLUS_BASE_URL = $TPGateway + '/v1'
    if ($electronRuntime) { $env:ELECTRON_RUN_AS_NODE = '1' }
    & $runtime $script | Out-Host
    if ($LASTEXITCODE -ne 0) { throw '配置未完成' }

    Write-Host ''
    Write-Host '  配置完成' -ForegroundColor Green
    Write-Host '  完全退出并重新打开 WorkBuddy，在模型选择器里选 TokenPlus 下的模型'
    TP-Exit 0
} catch {
    if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
    Write-Host ''
    Bad $_.Exception.Message
    Write-Host '  可以重新运行本工具再试一次'
    TP-Exit 1
} finally {
    $env:ELECTRON_RUN_AS_NODE = $previousRunAsNode
    Remove-Item Env:TOKENPLUS_API_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TOKENPLUS_BASE_URL -ErrorAction SilentlyContinue
    if ($tmp -and (Test-Path -LiteralPath $tmp)) { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}
