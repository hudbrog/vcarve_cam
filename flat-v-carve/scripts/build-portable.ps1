# Builds the portable Windows x64 command-line executable.
#
# The React web UI has been removed, so this script neither builds nor embeds UI
# assets. `cam serve` therefore needs --ui-dir with a prebuilt UI directory;
# the native GUI is cam-gui.exe.
#
# Package selection is --workspace in every Rust step of the validation pipeline
# so that Cargo resolves one feature set for shared dependencies. Mixing
# `--workspace` with `-p <single package>` invocations makes Cargo rebuild
# cam-app and cam-gui on every run.
param(
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$OutputDirectory = 'artifacts/portable',
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$camWorkspace = Split-Path -Parent $PSScriptRoot
if ($Target -ne 'x86_64-pc-windows-msvc') { throw 'This release script currently supports x86_64-pc-windows-msvc only.' }
$camOutput = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) { [System.IO.Path]::GetFullPath($OutputDirectory) } else { [System.IO.Path]::GetFullPath((Join-Path $camWorkspace $OutputDirectory)) }

Push-Location -LiteralPath $camWorkspace
$camPreviousFlags = $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
try {
    if ($env:RUSTFLAGS -or $env:CARGO_ENCODED_RUSTFLAGS) { throw 'Unset RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS so they cannot override the portable runtime flags.' }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
    $camBuildArgs = @('build', '--release', '--locked', '--workspace', '--bin', 'cam', '--target', $Target)
    if ($Offline) { $camBuildArgs += '--offline' }
    & cargo @camBuildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Portable executable build failed.' }
    # Use Cargo metadata so a configured CARGO_TARGET_DIR is respected.
    $camMetadataJson = & cargo metadata --no-deps --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Cargo target directory.' }
    $camMetadata = $camMetadataJson | ConvertFrom-Json
    $camExecutable = Join-Path $camMetadata.target_directory "$Target/release/cam.exe"
    New-Item -ItemType Directory -Path $camOutput -Force | Out-Null
    Copy-Item -LiteralPath $camExecutable -Destination (Join-Path $camOutput 'cam.exe')
    Get-Item -LiteralPath (Join-Path $camOutput 'cam.exe') | Select-Object FullName, Length
    Write-Host 'Portable CLI build ready. Run cam.exe --help, or cam-gui.exe for the native GUI.'
} finally {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = $camPreviousFlags
    Pop-Location
}
