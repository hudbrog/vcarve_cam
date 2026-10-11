param([string]$WasmBindgen='')
$ErrorActionPreference='Stop'
Push-Location $PSScriptRoot
try {
    $artifacts=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../artifacts/sketcher-feasibility'))
    node write-fixtures.mjs
    if($LASTEXITCODE -ne 0){throw 'Fixture generation failed'}
    cargo build --release --locked
    if($LASTEXITCODE -ne 0){throw 'Native Rust build failed'}
    & ./target/release/sketch-probe.exe fixtures.json "$artifacts/arael-native.json"
    if($LASTEXITCODE -ne 0){throw 'Native Arael run failed'}
    & ./target/release/sketch-probe.exe geometry | Set-Content "$artifacts/geometry-native.json" -Encoding utf8
    if($LASTEXITCODE -ne 0){throw 'Geometry qualification failed'}
    & ./build-native-plane.ps1
    & "$artifacts/native-plane/plane-probe.exe" fixtures.json "$artifacts/planegcs-native.json"
    if($LASTEXITCODE -ne 0){throw 'Native PlaneGCS run failed'}
    cargo test --release --locked
    if($LASTEXITCODE -ne 0){throw 'Geometry tests failed'}
    cargo build --release --lib --target wasm32-unknown-unknown --locked
    if($LASTEXITCODE -ne 0){throw 'Browser Rust build failed'}
    if(-not $WasmBindgen) {
        $bundled=Join-Path $PSScriptRoot '../../artifacts/wasm-bindgen-0.2.128-x86_64-pc-windows-msvc/wasm-bindgen.exe'
        if(Test-Path $bundled){$WasmBindgen=$bundled}else{$WasmBindgen=(Get-Command wasm-bindgen -ErrorAction Stop).Source}
    }
    $version=& $WasmBindgen --version
    if($version -ne 'wasm-bindgen 0.2.128'){throw "Expected wasm-bindgen 0.2.128, found $version"}
    & $WasmBindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/sketcher_feasibility.wasm
    if($LASTEXITCODE -ne 0){throw 'Browser bindings failed'}
    node lifecycle-native.mjs
    if($LASTEXITCODE -ne 0){throw 'Native lifecycle probe failed'}
    Write-Output 'Next: node serve.mjs, open http://127.0.0.1:5193/, run fixtures, then node verify.mjs'
} finally {Pop-Location}
