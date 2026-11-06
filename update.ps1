param([string]$Target = '', [string]$HerdrHome = '')
$ErrorActionPreference = 'Stop'
if (-not $HerdrHome) {
    $HerdrHome = Join-Path $env:USERPROFILE '.config\herdr-app'
    if ($env:HERDR_APP_HOME) { $HerdrHome = $env:HERDR_APP_HOME }
}
$destination = Join-Path $HerdrHome 'mods\zh-bridge'
if (-not (Test-Path -LiteralPath (Join-Path $destination 'mod.json'))) {
    throw 'Installed zh-bridge not found. Specify -HerdrHome with the actual configuration path.'
}
$pythonExe = (Get-Command python.exe -ErrorAction Stop).Source
& $pythonExe -m py_compile (Join-Path $PSScriptRoot 'bridge.py') (Join-Path $PSScriptRoot 'panel.py') (Join-Path $PSScriptRoot 'catalog.py') (Join-Path $PSScriptRoot 'widgets.py') (Join-Path $PSScriptRoot 'replies.py') (Join-Path $PSScriptRoot 'conversation.py') (Join-Path $PSScriptRoot 'platform_support.py') (Join-Path $PSScriptRoot 'tab_group.py')
if ($LASTEXITCODE -ne 0) { throw 'Python validation failed; update stopped.' }
$backup = Join-Path $destination ('.backup-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $backup | Out-Null
foreach ($name in @('bridge.py', 'panel.py', 'catalog.py', 'widgets.py', 'replies.py', 'conversation.py', 'platform_support.py', 'tab_group.py')) {
    if (Test-Path -LiteralPath (Join-Path $destination $name)) {
        Copy-Item -LiteralPath (Join-Path $destination $name) -Destination (Join-Path $backup $name)
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $destination $name) -Force
}
Write-Output "Updated. Backup: $backup"
$arguments = @(('"{0}"' -f (Join-Path $destination 'panel.py')))
if ($Target) {
    if ($Target -notmatch '^[A-Za-z0-9_./-]+$' -or $Target.StartsWith('-')) { throw 'Invalid terminal address.' }
    $arguments += @('--target', $Target)
}
Start-Process -FilePath $pythonExe -ArgumentList $arguments -WindowStyle Hidden | Out-Null
Write-Output 'Opened updated workspace. Use + beside the tabs to add a terminal. Drag a tab to the right to split, or onto a tab bar to merge.'
