# 多客户端一键配置（拼接在 common.ps1 之后）

$tools = @(
    @{ id = 'claude';         name = 'Claude Code';       needsModel = $true  },
    @{ id = 'claude-desktop'; name = 'Claude 桌面版';     needsModel = $false },
    @{ id = 'codex';          name = 'Codex';             needsModel = $true  },
    @{ id = 'grok';           name = 'Grok Build';        needsModel = $true  },
    @{ id = 'deepseek';       name = 'DeepSeek Harness';  needsModel = $true  },
    @{ id = 'workbuddy';      name = 'WorkBuddy';         needsModel = $false }
)

function TP-ToolInstalled($id) {
    switch ($id) {
        'claude' {
            if (Get-Command claude -ErrorAction SilentlyContinue) { return $true }
            return (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.claude'))
        }
        'claude-desktop' {
            foreach ($p in @((Join-Path $env:LOCALAPPDATA 'Programs\Claude\Claude.exe'), (Join-Path $env:LOCALAPPDATA 'AnthropicClaude\Claude.exe'))) {
                if (Test-Path -LiteralPath $p) { return $true }
            }
            return (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'Claude-3p'))
        }
        'codex' {
            if (Get-Command codex -ErrorAction SilentlyContinue) { return $true }
            return (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.codex'))
        }
        'grok' {
            if (Get-Command grok -ErrorAction SilentlyContinue) { return $true }
            return (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.grok'))
        }
        'deepseek' {
            $candidates = @("$env:LOCALAPPDATA\Programs\DeepSeek Harness")
            if ($env:ProgramFiles) { $candidates += "$env:ProgramFiles\DeepSeek Harness" }
            foreach ($root in $candidates) {
                if (Test-Path -LiteralPath $root) { return $true }
            }
            return $false
        }
        'workbuddy' {
            $candidates = @((Join-Path $env:LOCALAPPDATA 'Programs\WorkBuddy'), (Join-Path $env:LOCALAPPDATA 'Programs\WorkBuddy AI'))
            if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'WorkBuddy') }
            foreach ($root in $candidates) {
                if (Test-Path -LiteralPath $root) { return $true }
            }
            return (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.workbuddy'))
        }
    }
    return $false
}

Write-Host ''
Write-Host '   TokenPlus · 多客户端一键配置' -ForegroundColor Cyan
Write-Host '   一条命令，把 AI 编程工具接入 TokenPlus'
Write-Host ''

foreach ($t in $tools) { $t.found = (TP-ToolInstalled $t.id); $t.picked = $t.found }

if ($env:TOKENPLUS_TOOLS) {
    foreach ($t in $tools) { $t.picked = ((',' + $env:TOKENPLUS_TOOLS + ',') -like ('*,' + $t.id + ',*')) }
} elseif (TP-CanInteract) {
    # 多选：↑↓ 移动，空格勾选，a 全选/全不选，回车确认
    $cursor = 0; $drawn = 0
    $width = 70
    try { $width = [Math]::Max(30, [Console]::WindowWidth - 2) } catch {}
    $prevVisible = $true
    try { $prevVisible = [Console]::CursorVisible; [Console]::CursorVisible = $false } catch {}
    try {
        while ($true) {
            if ($drawn -gt 0) {
                try { [Console]::SetCursorPosition(0, [Math]::Max(0, [Console]::CursorTop - $drawn)) } catch {}
            }
            $lines = New-Object System.Collections.Generic.List[string]
            $lines.Add('  ? 要配置哪些客户端 (↑↓ 移动 · 空格勾选 · a 全选 · 回车确认)')
            for ($i = 0; $i -lt $tools.Count; $i++) {
                $t = $tools[$i]
                $box = '◻'; if ($t.picked) { $box = '◼' }
                $tag = '未检测到'; if ($t.found) { $tag = '已安装' }
                $ptr = '  '; if ($i -eq $cursor) { $ptr = '❯ ' }
                $lines.Add('  ' + $ptr + $box + ' ' + $t.name + '  ' + $tag)
            }
            foreach ($line in $lines) {
                $out = $line
                if ($out.Length -gt $width) { $out = $out.Substring(0, $width) }
                Write-Host ($out.PadRight($width))
            }
            $drawn = $lines.Count
            $key = [Console]::ReadKey($true)
            switch ($key.Key) {
                'Enter' { break }
                'Spacebar' { $tools[$cursor].picked = -not $tools[$cursor].picked }
                'UpArrow' { if ($cursor -gt 0) { $cursor-- } }
                'DownArrow' { if ($cursor -lt ($tools.Count - 1)) { $cursor++ } }
                default {
                    if ($key.KeyChar -eq 'a' -or $key.KeyChar -eq 'A') {
                        $allOn = $true
                        foreach ($t in $tools) { if (-not $t.picked) { $allOn = $false } }
                        foreach ($t in $tools) { $t.picked = (-not $allOn) }
                    }
                }
            }
            if ($key.Key -eq 'Enter') { break }
        }
    } finally {
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
}

$selected = @($tools | Where-Object { $_.picked })
if ($selected.Count -eq 0) { Write-Host '  没有选择任何客户端，已退出'; TP-Exit 1 }
Write-Host ('  将配置：' + (($selected | ForEach-Object { $_.name }) -join '、'))

TP-CheckGateway
TP-RequireKey
if (@($selected | Where-Object { $_.needsModel }).Count -gt 0) { TP-RequireModel }

# 逐个下载对应工具的独立脚本，用子进程运行；key 和模型通过环境变量传下去，不会重复提问
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('tokenplus-setup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
$env:TOKENPLUS_API_KEY = $script:TPApiKey
if ($script:TPModel) { $env:TOKENPLUS_MODEL = $script:TPModel }
$okList = @(); $failList = @()
try {
    foreach ($t in $selected) {
        Write-Host ''
        Write-Host ('  ===== ' + $t.name + ' =====') -ForegroundColor Cyan
        $file = Join-Path $tmp ($t.id + '-setup.ps1')
        try {
            TP-FetchAsset ($t.id + '-setup.ps1') $file
        } catch {
            if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
            $failList += $t.name
            Note ('配置工具下载失败：' + $_.Exception.Message)
            continue
        }
        $env:TOKENPLUS_NO_OPEN = '1'
        # 用当前正在运行的 PowerShell 本体执行（Windows 上是 powershell.exe）
        $psExe = (Get-Process -Id $PID).Path
        & $psExe -NoProfile -ExecutionPolicy Bypass -File $file
        if ($LASTEXITCODE -eq 0) { $okList += $t.name } else { $failList += $t.name }
    }
} finally {
    Remove-Item Env:TOKENPLUS_API_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TOKENPLUS_MODEL -ErrorAction SilentlyContinue
    Remove-Item Env:TOKENPLUS_NO_OPEN -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
if ($failList.Count -eq 0) {
    Write-Host '  全部配置完成' -ForegroundColor Green
    TP-Exit 0
}
Write-Host ('  完成 ' + $okList.Count + ' 个，失败 ' + $failList.Count + ' 个：' + ($failList -join '、')) -ForegroundColor Yellow
Write-Host '  失败的可以单独重新运行对应客户端的配置命令'
TP-Exit 1
