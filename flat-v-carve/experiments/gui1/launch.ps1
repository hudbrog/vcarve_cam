param([ValidateSet('native','web','dom')][string]$Target = 'native')
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if ($Target -eq 'native') {
        cargo build --release --locked --bin cam-gui1-desktop
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
        & ./target/release/cam-gui1-desktop.exe
    } else {
        wasm-pack build --target web --release --out-dir pkg --out-name cam_gui1 -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Browser build failed' }
        node web/write-offline-manifest.mjs
        if ($LASTEXITCODE -ne 0) { throw 'Offline manifest failed' }
        Write-Host 'Review: http://127.0.0.1:5181/web/index.html'
        Write-Host 'DOM boundary comparison: http://127.0.0.1:5181/web/dom-probe.html'
        node web/serve.mjs
    }
} finally { Pop-Location }
