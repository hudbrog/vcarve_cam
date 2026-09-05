#requires -Version 7.0
param([string]$OutputDirectory = 'artifacts/m5')
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$camPath = Join-Path $workspace 'target/release/cam.exe'
if (-not (Test-Path -LiteralPath $camPath)) { throw 'Build first: cargo build --release --workspace --locked' }
$destination = [IO.Path]::GetFullPath($OutputDirectory, $workspace)
New-Item -ItemType Directory -Path $destination -Force | Out-Null

function Invoke-MeasuredCam([string[]]$CamArguments, [string]$LogPrefix) {
    $info = [Diagnostics.ProcessStartInfo]::new($camPath)
    $info.WorkingDirectory = $workspace
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $CamArguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $peak = 0L
    try {
        [void]$process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        while (-not $process.WaitForExit(20)) {
            try { $process.Refresh(); $peak = [Math]::Max($peak, $process.PeakWorkingSet64) } catch { }
        }
        $process.WaitForExit()
        $timer.Stop()
        [IO.File]::WriteAllText($LogPrefix + '.stdout.log', $stdout.GetAwaiter().GetResult())
        [IO.File]::WriteAllText($LogPrefix + '.stderr.log', $stderr.GetAwaiter().GetResult())
        return [pscustomobject]@{
            exit_code = $process.ExitCode
            elapsed_seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 4)
            observed_peak_working_set_bytes = $peak
        }
    } finally { $process.Dispose() }
}

$cases = Get-Content (Join-Path $workspace 'fixtures/m5/cases.json') -Raw | ConvertFrom-Json
$results = @()
foreach ($case in $cases) {
    $folder = Join-Path $destination $case.id
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    $planPath = Join-Path $folder 'plan.json'
    $reportPath = Join-Path $folder 'verification.json'
    $previewPath = Join-Path $folder 'verification.svg'
    $planning = Invoke-MeasuredCam -CamArguments @('plan', $case.job, '--output', $planPath) -LogPrefix (Join-Path $folder 'planning')
    if ($planning.exit_code -ne 0) { throw "Planning failed for $($case.id); see its planning.stderr.log" }
    $arguments = @('verify', $planPath, '--output', $reportPath, '--preview', $previewPath) + @($case.arguments)
    $verification = Invoke-MeasuredCam -CamArguments $arguments -LogPrefix (Join-Path $folder 'verification')
    $data = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
    $expectedExit = if ($case.expected -eq 'passed') { 0 } else { 1 }
    $passed = $data.verification.status -eq $case.expected -and $verification.exit_code -eq $expectedExit
    $results += [pscustomobject]@{
        id = $case.id
        expected = $case.expected
        actual = $data.verification.status
        expectation_met = $passed
        planning = $planning
        verification = $verification
        motion_count = $plan.endmill.motions.Count + $plan.vbit_motions.Count
        original_cells = $data.verification.original.evaluated_cells
        rounded_cells = $data.verification.rounded.verification.evaluated_cells
        bounds = $data.verification.original.bounds
        plan_bytes = (Get-Item -LiteralPath $planPath).Length
        report_bytes = (Get-Item -LiteralPath $reportPath).Length
        preview_bytes = (Get-Item -LiteralPath $previewPath).Length
    }
    Write-Output "$($case.id): $($data.verification.status), plan $($planning.elapsed_seconds)s, verify $($verification.elapsed_seconds)s"
}
$summary = [ordered]@{
    measured_at = [DateTimeOffset]::Now.ToString('o')
    runtime = $PSVersionTable.PSVersion.ToString()
    platform = [Environment]::OSVersion.VersionString
    memory_measurement = 'OS peak working set observed every 20 ms; short-lived/final spikes can be missed. Zero means no observation.'
    verification_measurement = 'Includes saved-plan authentication/replay, original/optional rounded verification, JSON and SVG output.'
    cases = $results
}
$summary | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $destination 'benchmark.json') -Encoding utf8NoBOM
if ($results.Where({ -not $_.expectation_met }).Count -ne 0) { throw 'M5 fixture expectations failed; see benchmark.json and per-case reports.' }
