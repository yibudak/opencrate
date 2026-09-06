#Requires -Version 5.1
# Gather license texts from the exact Cargo dependency graph used for this target.
[CmdletBinding()]
param([Parameter(Mandatory)]$Metadata, [Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$supplements = Get-Content (Join-Path $projectRoot 'installer/licenses/supplements.json') -Raw | ConvertFrom-Json
$packages = @{}
foreach ($package in $Metadata.packages) { $packages[$package.id] = $package }
$seen = [Collections.Generic.HashSet[string]]::new()
$tree = & cargo tree --locked --offline -p opencrate-ui --target x86_64-pc-windows-gnu --edges normal,build --prefix none --format '{p}'
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve enabled dependency licenses.' }
foreach ($line in $tree) {
    if ($line -match '^([a-zA-Z0-9_-]+) v([^\s]+)') {
        $name = $Matches[1]
        $version = $Matches[2]
        foreach ($package in $Metadata.packages | Where-Object { $_.name -eq $name -and $_.version -eq $version }) {
            [void]$seen.Add($package.id)
        }
    }
}
$text = [Text.StringBuilder]::new()
[void]$text.AppendLine('OpenCrate - Third-party notices')
[void]$text.AppendLine('Includes runtime and build-time dependencies from Cargo.lock. The license texts below retain their original wording.')
$missing = @()
foreach ($package in ($seen | ForEach-Object { $packages[$_] } | Sort-Object name, version)) {
    if (-not $package.source) { continue }
    $root = Split-Path -Parent $package.manifest_path
    [void]$text.AppendLine("`n=== $($package.name) $($package.version) ===")
    [void]$text.AppendLine("License: $($package.license)")
    [void]$text.AppendLine("Source: $($package.repository)")
    $files = @(Get-ChildItem -LiteralPath $root -Recurse -File | Where-Object {
        $_.Name.ToUpperInvariant() -cmatch '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)([._-]|$)' -or
        $_.Name.ToUpperInvariant() -cmatch '^(OFL|UFL|HACK-REGULAR)\.TXT$' -or $_.Name.ToUpperInvariant() -cmatch '-LICENSE\.TXT$'
    } | Sort-Object FullName)
    if ($files.Count -eq 0) {
        $entry = $supplements.PSObject.Properties["$($package.name)@$($package.version)"]
        if ($entry -and (Test-Path -LiteralPath (Join-Path $projectRoot "installer/licenses/$($entry.Value)"))) {
            [void]$text.AppendLine("License text: $($entry.Value) (included below)")
        }
        else { $missing += "$($package.name) $($package.version)" }
    }
    foreach ($file in $files) {
        [void]$text.AppendLine("`n--- $($file.Name) ---")
        [void]$text.AppendLine([IO.File]::ReadAllText($file.FullName))
    }
}
foreach ($file in (Get-ChildItem (Join-Path $projectRoot 'installer/licenses') -Filter '*.txt' -File | Sort-Object Name)) {
    [void]$text.AppendLine("`n=== $($file.Name) ===")
    [void]$text.AppendLine([IO.File]::ReadAllText($file.FullName))
}
[void]$text.AppendLine("`n=== Noto Sans SC (embedded fallback font) ===")
[void]$text.AppendLine([IO.File]::ReadAllText((Join-Path $projectRoot 'assets/fonts/OFL-NotoSansSC.txt')))
[IO.File]::WriteAllText($OutputPath, $text.ToString(), [Text.UTF8Encoding]::new($false))
if ($missing.Count) { throw "Missing license texts; add supplements in installer/licenses: $($missing -join ', ')" }
