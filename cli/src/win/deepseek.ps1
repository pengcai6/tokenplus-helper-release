# DeepSeek Harness 工具逻辑（拼接在 common.ps1 之后）

TP-Banner 'DeepSeek Harness 一键配置'
TP-CheckGateway

# Resolve only known app locations; never scan drives or execute registry commands.
$installs = New-Object 'System.Collections.Generic.List[object]'
function Add-Installation([string]$candidate) {
    try {
        if ([string]::IsNullOrWhiteSpace($candidate)) { return }
        $candidate = [Environment]::ExpandEnvironmentVariables($candidate.Trim().Trim('"'))
        if (-not [IO.Path]::IsPathRooted($candidate)) { return }
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $candidate = [IO.Path]::GetDirectoryName($candidate) }
        if (-not (Test-Path -LiteralPath $candidate -PathType Container)) { return }
        $root = (Get-Item -LiteralPath $candidate).FullName
        $exe = $null; $app = $null
        foreach ($name in @('DeepSeek Harness.exe', 'deepseek-harness-desktop.exe')) {
            $file = Join-Path $root $name
            if (Test-Path -LiteralPath $file -PathType Leaf) { $exe = $file; break }
        }
        foreach ($name in @('resources\app.asar', 'resources\app')) {
            $file = Join-Path $root $name
            if (($name.EndsWith('.asar') -and (Test-Path -LiteralPath $file -PathType Leaf)) -or
                (-not $name.EndsWith('.asar') -and (Test-Path -LiteralPath $file -PathType Container))) { $app = $file; break }
        }
        if (-not $exe -or -not $app) { return }
        foreach ($install in $installs) { if ($install.Root -eq $root) { return } }
        $installs.Add([pscustomobject]@{ Root = $root; Exe = $exe; App = $app })
    } catch {
        if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
        # Ignore stale, inaccessible or malformed discovery candidates.
        return
    }
}
function Add-DisplayIcon([string]$icon) {
    if ([string]::IsNullOrWhiteSpace($icon)) { return }
    if ($icon -match '^\s*"([^"]+)"') { Add-Installation $Matches[1] }
    else { Add-Installation ($icon -replace ',\s*-?\d+\s*$', '') }
}
# Windows CRT quoting: double trailing slashes, and use 2n+1 before quotes.
function Quote-WindowsArgument([string]$value) {
    $quoted = New-Object Text.StringBuilder
    [void]$quoted.Append('"'); $slashes = 0
    foreach ($character in $value.ToCharArray()) {
        if ($character -eq [char]92) { $slashes++; continue }
        if ($character -eq [char]34) {
            [void]$quoted.Append([char]92, (2 * $slashes + 1))
        } elseif ($slashes -gt 0) {
            [void]$quoted.Append([char]92, $slashes)
        }
        [void]$quoted.Append($character); $slashes = 0
    }
    if ($slashes -gt 0) { [void]$quoted.Append([char]92, (2 * $slashes)) }
    [void]$quoted.Append('"')
    return $quoted.ToString()
}

try {
    foreach ($process in @(Get-Process -Name 'DeepSeek Harness', 'deepseek-harness-desktop' -ErrorAction SilentlyContinue)) {
        try { Add-Installation $process.Path } catch {
            # Other users' processes may deny access.
        }
    }
    foreach ($key in @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )) {
        foreach ($entry in @(Get-ItemProperty -Path $key -ErrorAction SilentlyContinue)) {
            if ($entry.DisplayName -match '(?i)^DeepSeek(?: Harness|[- ]harness[- ]desktop)?(?:\s|$)') {
                Add-Installation $entry.InstallLocation
                Add-DisplayIcon $entry.DisplayIcon
            }
        }
    }
    foreach ($root in @(
        "$env:LOCALAPPDATA\Programs\DeepSeek Harness",
        "$env:LOCALAPPDATA\Programs\deepseek-harness-desktop",
        "$env:ProgramFiles\DeepSeek Harness",
        ([IO.Path]::Combine([Environment]::GetFolderPath('ProgramFilesX86'), 'DeepSeek Harness'))
    )) {
        Add-Installation $root
    }
    while ($installs.Count -eq 0) {
        Write-Host '  未自动找到 DeepSeek Harness。支持安装到任意盘，请输入安装文件夹或程序 exe 的完整路径。'
        $manual = Read-Host '  安装路径（直接回车取消）'
        if ([string]::IsNullOrWhiteSpace($manual)) { throw '已取消，未修改配置' }
        Add-Installation $manual
        if ($installs.Count -eq 0) { Write-Host '  路径无效：需要同时包含 DeepSeek 程序及 resources/app.asar 或 resources/app，请重新输入。' -ForegroundColor Yellow }
    }
    $selected = $installs[0]
    if ($installs.Count -gt 1) {
        Write-Host '  检测到多个 DeepSeek 安装，请选择本次配置使用的版本：'
        for ($i = 0; $i -lt $installs.Count; $i++) { Write-Host ('  ' + ($i + 1) + '. ' + $installs[$i].Root) }
        while ($true) {
            $answer = Read-Host '  输入序号（直接回车取消）'; $number = 0
            if ([string]::IsNullOrWhiteSpace($answer)) { throw '已取消，未修改配置' }
            if ([int]::TryParse($answer, [ref]$number) -and $number -ge 1 -and $number -le $installs.Count) { $selected = $installs[$number - 1]; break }
            Write-Host '  请输入列表中的有效序号。' -ForegroundColor Yellow
        }
    }
    $appRoot = $selected.App; $exe = $selected.Exe
    Write-Host ('  使用 DeepSeek 安装目录：' + $selected.Root)

    TP-RequireKey
    TP-RequireModel

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('tokenplus-dsh-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    $child = $null; $started = $false; $exited = $false
    try {
        $script = Join-Path $tmp 'deepseek-config.mjs'
        TP-FetchAsset 'deepseek-config.mjs' $script
        if (-not (Test-Path -LiteralPath $script -PathType Leaf) -or (Get-Item -LiteralPath $script).Length -eq 0) { throw '配置程序下载失败或为空，请重新运行' }
        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = $exe
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.StandardOutputEncoding = [Text.Encoding]::UTF8
        $startInfo.StandardErrorEncoding = [Text.Encoding]::UTF8
        $startInfo.WorkingDirectory = $selected.Root
        $arguments = @($script, $appRoot) | ForEach-Object { Quote-WindowsArgument $_ }
        $startInfo.Arguments = $arguments -join ' '
        # Set only the child's environment; preserve the caller's existing value.
        # key 走环境变量，不出现在进程参数里
        $startInfo.EnvironmentVariables['ELECTRON_RUN_AS_NODE'] = '1'
        $startInfo.EnvironmentVariables['TOKENPLUS_API_KEY'] = $script:TPApiKey
        $startInfo.EnvironmentVariables['TOKENPLUS_MODEL'] = $script:TPModel
        $startInfo.EnvironmentVariables['TOKENPLUS_MODELS'] = [string]::Join(' ', $script:TPModels)
        $startInfo.EnvironmentVariables['TOKENPLUS_BASE_URL'] = ($TPGateway + '/v1')
        $child = New-Object System.Diagnostics.Process
        $child.StartInfo = $startInfo
        $started = $child.Start()
        if (-not $started) { throw '无法启动 DeepSeek 配置程序' }
        # Drain both pipes concurrently so verbose errors cannot block the child.
        Write-Host '  正在配置 DeepSeek，请稍候，不要关闭此窗口...'
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        $child.WaitForExit()
        $exited = $child.HasExited
        $output = $stdout.GetAwaiter().GetResult()
        $errorOutput = $stderr.GetAwaiter().GetResult()
        if ($output) { [Console]::Write($output) }
        if ($errorOutput) { [Console]::Error.Write($errorOutput) }
        if (-not $exited) { throw 'DeepSeek 配置程序尚未退出，未清理临时文件' }
        if ($child.ExitCode -ne 0) { throw ('配置未完成，退出码：' + $child.ExitCode) }
        Write-Host ''
        Write-Host ('  配置完成，默认模型 ' + $script:TPModel) -ForegroundColor Green
        Write-Host '  完全退出 DeepSeek Harness 后重新打开，新建对话即可使用'
    } finally {
        # Never delete the script while a GUI-subsystem Electron child is using it.
        if ($started -and -not $exited) {
            try { $child.WaitForExit(); $exited = $child.HasExited } catch { $exited = $false }
        }
        if (-not $started -or $exited) {
            Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
            if ($null -ne $child) { $child.Dispose() }
        } else {
            Write-Host '  配置进程仍在运行，已保留临时文件；请等待完成。' -ForegroundColor Yellow
        }
    }
    TP-Exit 0
} catch {
    if ($_.Exception.Message -eq $script:TPSentinel) { throw $_ }
    Write-Host ''
    Bad $_.Exception.Message
    Write-Host '  可以重新运行本工具再试一次'
    TP-Exit 1
}
