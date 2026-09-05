param([string]$OutputDirectory = 'artifacts/m6', [string[]]$CaseId = @())
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $workspace
$cam = Join-Path $workspace 'target/release/cam.exe'
if (!(Test-Path -LiteralPath $cam)) { throw 'Build first: cargo build --release --locked --workspace' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new output directory; prior exports are preserved.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$cases = Get-Content -LiteralPath 'fixtures/m6/cases.json' -Raw | ConvertFrom-Json
if ($CaseId.Count -gt 0) {
    foreach ($id in $CaseId) { if ($id -notin $cases.id) { throw "Unknown M6 case: $id" } }
    $cases = @($cases | Where-Object { $_.id -in $CaseId })
}
$summary = @()
foreach ($case in $cases) {
    $caseDirectory = Join-Path $OutputDirectory $case.id
    New-Item -ItemType Directory -Path $caseDirectory | Out-Null
    $plan = Join-Path $caseDirectory 'plan.json'
    & $cam plan "fixtures/m4/$($case.job).json" --output $plan 2> (Join-Path $caseDirectory 'plan.log')
    if ($LASTEXITCODE -ne 0) { throw "Plan failed: $($case.id)" }
    $profile = Get-Content -LiteralPath "fixtures/m6/$($case.profile).json" -Raw | ConvertFrom-Json
    if ($case.PSObject.Properties.Name -contains 'decimal_places') { $profile.decimal_places = $case.decimal_places }
    $profilePath = Join-Path $caseDirectory 'profile.json'
    $profile | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $profilePath -Encoding utf8
    $export = Join-Path $caseDirectory 'export'
    $exportArgs = @('export', $plan, '--profile', $profilePath, '--layout', $case.layout, '--output', $export)
    if ($case.max_cells) { $exportArgs += @('--max-cells', [string]$case.max_cells) }
    & $cam @exportArgs 2> (Join-Path $caseDirectory 'export.log')
    $code = $LASTEXITCODE
    $report = Get-Content -LiteralPath (Join-Path $export 'export-report.json') -Raw | ConvertFrom-Json
    if ($report.status -ne $case.status -or $code -ne $(if ($case.status -eq 'passed') { 0 } else { 1 })) {
        throw "Unexpected export result: $($case.id), $($report.status), exit $code"
    }
    $bytes = 0
    if ($case.status -eq 'passed') {
        $readArgs = @('verify-gcode', $plan, '--profile', $profilePath, '--layout', $case.layout, '--output', (Join-Path $caseDirectory 'readback.json'))
        foreach ($program in $report.programs) {
            $file = Join-Path $export $program.filename
            $bytes += (Get-Item -LiteralPath $file).Length
            $readArgs += @('--program', $file)
        }
        & $cam @readArgs 2> (Join-Path $caseDirectory 'readback.log')
        if ($LASTEXITCODE -ne 0) { throw "Saved program readback failed: $($case.id)" }
        $readback = Get-Content -LiteralPath (Join-Path $caseDirectory 'readback.json') -Raw | ConvertFrom-Json
        if (($readback.programs.sha256 -join ',') -ne ($report.programs.sha256 -join ',')) { throw 'Readback hash mismatch' }
    } elseif (Get-ChildItem -LiteralPath $export -Filter '*.ngc') { throw 'A failed export published G-code' }
    $summary += [ordered]@{id=$case.id; status=$report.status; programs=@($report.programs).Count; motions=($report.programs.motion_count | Measure-Object -Sum).Sum; gcode_bytes=$bytes}
    Write-Host "$($case.id): $($report.status); $bytes G-code bytes"
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'summary.json') -Encoding utf8
Write-Host "All $($cases.Count) M6 fixture expectations passed."
