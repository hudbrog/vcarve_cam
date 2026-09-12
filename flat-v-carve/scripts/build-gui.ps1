param(
    [ValidateSet('native','web')][string]$Target = 'native',
    # --no-opt skips wasm-opt for the browser build: measured 1.3s instead of
    # ~30s, at 9.9 MB instead of 8.35 MB of wasm. Keep optimization for
    # artifacts that ship; use this for review/CI builds where speed matters.
    [switch]$NoOpt,
    [switch]$Launch
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
Push-Location $workspace
try {
    if ($Target -eq 'native') {
        # --workspace keeps Cargo's feature resolution identical to the clippy and
        # test steps. A bare `-p cam-gui` resolves shared dependencies with a
        # different feature set, which makes every later workspace Cargo command
        # rebuild cam-gui and cam-app (UnitDependencyInfoChanged).
        cargo build --workspace --release --locked --bin cam-gui
        if ($LASTEXITCODE -ne 0) { throw 'GUI native build failed' }
        $metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
        $buildRoot = $metadata.target_directory
        if ($env:CARGO_BUILD_TARGET) { $buildRoot = Join-Path $buildRoot $env:CARGO_BUILD_TARGET }
        $destination = Join-Path $workspace 'artifacts/gui/native'
        New-Item -ItemType Directory -Force $destination | Out-Null
        $executable = Join-Path $destination 'cam-gui.exe'
        Copy-Item -LiteralPath (Join-Path $buildRoot 'release/cam-gui.exe') -Destination $executable -Force
        Copy-Item -LiteralPath 'crates/cam-gui/licenses' -Destination $destination -Recurse -Force
        Copy-Item -LiteralPath 'crates/cam-gui/THIRD-PARTY.md' -Destination $destination -Force
        $hash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash *cam-gui.exe" | Set-Content (Join-Path $destination 'SHA256SUMS') -Encoding utf8NoBOM
        Write-Host "GUI: $executable"
        if ($Launch) { Start-Process -FilePath $executable }
    } else {
        Push-Location (Join-Path $workspace 'crates/cam-gui')
        try {
            $wasmPackArgs = @('build', '--target', 'web', '--release', '--out-dir', 'pkg', '--out-name', 'cam_gui')
            if ($NoOpt) { $wasmPackArgs += '--no-opt' }
            $wasmPackArgs += @('--', '--locked')
            & wasm-pack @wasmPackArgs
            if ($LASTEXITCODE -ne 0) { throw 'GUI browser build failed' }
            node web/write-offline-manifest.mjs
            if ($LASTEXITCODE -ne 0) { throw 'Offline manifest failed' }
            $destination = Join-Path $workspace 'artifacts/gui/browser'
            New-Item -ItemType Directory -Force (Join-Path $destination 'web') | Out-Null
            Copy-Item -LiteralPath 'pkg' -Destination $destination -Recurse -Force
            Copy-Item -LiteralPath 'licenses' -Destination $destination -Recurse -Force
            Copy-Item -LiteralPath 'THIRD-PARTY.md' -Destination $destination -Force
            foreach ($asset in @('index.html','worker.js','recovery-store.js','sw.js','offline-manifest.js')) {
                Copy-Item -LiteralPath (Join-Path 'web' $asset) -Destination (Join-Path $destination 'web') -Force
            }
            Write-Host 'GUI: http://127.0.0.1:5182/web/index.html'
            if ($Launch) { node web/serve.mjs }
        } finally { Pop-Location }
    }
} finally { Pop-Location }
