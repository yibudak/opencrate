#Requires -Version 5.1
<#
.SYNOPSIS
Build a self-contained x64 Windows installer and SHA-256 checksum.
.DESCRIPTION
Builds the Windows GNU target with stable Rust and Inno Setup 6.7.3.
BootstrapInnoSetup downloads a checksum-pinned compiler into target only;
portable mode does not register the compiler or modify PATH/file associations.
#>
[CmdletBinding()]
param(
    [switch]$BootstrapInnoSetup,
    [switch]$SkipBuild,
    [string]$IsccPath,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'dist' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$toolRoot = Join-Path $projectRoot 'target/installer-tools'
$portableRoot = Join-Path $toolRoot 'inno-6.7.3'

if (-not $IsccPath) {
    $candidates = @(
        (Join-Path $portableRoot 'ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6/ISCC.exe')
    )
    # Bootstrap always selects the pinned portable compiler, including on CI.
    if ($BootstrapInnoSetup) { $candidates = @((Join-Path $portableRoot 'ISCC.exe')) }
    $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $IsccPath -and $BootstrapInnoSetup) {
    New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null
    $download = Join-Path $toolRoot 'innosetup-6.7.3.exe'
    $expectedHash = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'
    if (-not (Test-Path -LiteralPath $download)) {
        Invoke-WebRequest 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $download -UseBasicParsing
    }
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $expectedHash) {
        throw 'Inno Setup checksum mismatch. Remove the cached download and retry.'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $download
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'CN=Pyrsys B\.V\.') {
        throw 'Inno Setup publisher signature could not be verified.'
    }
    $bootstrap = Start-Process -FilePath $download -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/PORTABLE=1',
        ('/DIR="' + $portableRoot + '"'), ('/LOG="' + (Join-Path $toolRoot 'bootstrap.log') + '"')
    ) -WindowStyle Hidden -Wait -PassThru
    if ($bootstrap.ExitCode -ne 0) { throw "Inno Setup bootstrap failed: $($bootstrap.ExitCode)" }
    $IsccPath = Join-Path $portableRoot 'ISCC.exe'
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) {
    throw 'Inno Setup compiler not found. Use -BootstrapInnoSetup or -IsccPath.'
}

Push-Location $projectRoot
try {
    $metadataJson = & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-gnu
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $app = $metadata.packages | Where-Object name -eq 'opencrate-ui'
    $version = $app.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Installer requires a numeric major.minor.patch version.' }
    if (-not $SkipBuild) {
        $oldRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
        $oldCFlags = $env:CFLAGS
        $oldCxxFlags = $env:CXXFLAGS
        $oldShellEscapedFlags = $env:CC_SHELL_ESCAPED_FLAGS
        try {
            $profileRoot = [Environment]::GetFolderPath('UserProfile')
            $cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $profileRoot '.cargo' }
            $rustupRoot = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $profileRoot '.rustup' }
            $mappings = @(
                @($profileRoot, '/build-user'), @($cargoRoot, '/cargo'),
                @($rustupRoot, '/rustup'), @($projectRoot, '/opencrate')
            )
            $flags = @()
            if ($oldRustFlags) { $flags += $oldRustFlags.Split([char]31) }
            $nativeFlags = @()
            foreach ($mapping in $mappings) {
                foreach ($prefix in @($mapping[0], $mapping[0].Replace('\', '/')) | Select-Object -Unique) {
                    $flags += "--remap-path-prefix=$prefix=$($mapping[1])"
                    $escapedPrefix = $prefix.Replace('\', '\\').Replace('"', '\"')
                    $nativeFlags += ('"-ffile-prefix-map=' + $escapedPrefix + '=' + $mapping[1] + '"')
                }
            }
            $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
            $env:CFLAGS = (@($oldCFlags) + $nativeFlags) -join ' '
            $env:CXXFLAGS = (@($oldCxxFlags) + $nativeFlags) -join ' '
            $env:CC_SHELL_ESCAPED_FLAGS = '1'
            & cargo build --locked --release --target x86_64-pc-windows-gnu -p opencrate-ui
            if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        }
        finally {
            $env:CARGO_ENCODED_RUSTFLAGS = $oldRustFlags
            $env:CFLAGS = $oldCFlags
            $env:CXXFLAGS = $oldCxxFlags
            $env:CC_SHELL_ESCAPED_FLAGS = $oldShellEscapedFlags
        }
    }
    $binary = Join-Path $metadata.target_directory 'x86_64-pc-windows-gnu/release/opencrate-ui.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Release executable missing. Run without -SkipBuild.' }
    if ((Get-Item -LiteralPath $binary).VersionInfo.ProductVersion -ne $version) {
        throw 'Executable version does not match Cargo.toml. Rebuild before packaging.'
    }
    # Check the PE architecture independently of the build directory name.
    $pe = [IO.File]::ReadAllBytes($binary)
    $peOffset = [BitConverter]::ToInt32($pe, 0x3c)
    if ([BitConverter]::ToUInt16($pe, $peOffset + 4) -ne 0x8664) { throw 'Expected an x64 executable.' }

    $payload = Join-Path $projectRoot 'target/installer-payload'
    New-Item -ItemType Directory -Force -Path $payload, $OutputDirectory | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $payload 'OpenCrate.exe') -Force
    Copy-Item -LiteralPath (Join-Path $projectRoot 'README.md') -Destination $payload -Force
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $payload 'LICENSE.txt') -Force
    & (Join-Path $PSScriptRoot 'generate-notices.ps1') -Metadata $metadata -OutputPath (Join-Path $payload 'THIRD-PARTY-NOTICES.txt')

    & python (Join-Path $PSScriptRoot 'check-privacy.py') --payload $payload
    if ($LASTEXITCODE -ne 0) { throw 'Installer payload privacy check failed.' }

    & $IsccPath '/Qp' "/DAppVersion=$version" "/DPayloadDir=$payload" "/DOutputDir=$OutputDirectory" (Join-Path $projectRoot 'installer/opencrate.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $setup = Join-Path $OutputDirectory "OpenCrate-$version-windows-x64-setup.exe"
    $hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$setup.sha256", "$hash  $([IO.Path]::GetFileName($setup))`n", [Text.UTF8Encoding]::new($false))
    Write-Output "Installer: $setup"
    Write-Output "SHA-256: $hash"
}
finally { Pop-Location }
