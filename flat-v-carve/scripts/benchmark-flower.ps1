param(
    [string]$Job = '../real_data/flower_box-svg.job (2).json',
    [string]$OutputDirectory = 'artifacts/flower-performance',
    [string]$Cam = '',
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 300,
    [ValidateSet('endmill', 'combined')][string[]]$Stages = @('endmill', 'combined')
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
$jobPath = (Resolve-Path -LiteralPath $Job).Path
$camPath = if ($Cam) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Cam) } else { Join-Path $workspace 'target/release/cam.exe' }
if (-not (Test-Path -LiteralPath $camPath)) { throw 'Build first: cargo build --release --locked -p cam-app' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new output directory.' }
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$results = @()
foreach ($stage in $Stages) {
    $planPath = Join-Path $outputPath "$stage.plan.json"
    $info = [Diagnostics.ProcessStartInfo]::new($camPath)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.Environment['CAM_TIMINGS'] = '1'
    foreach ($arg in @('plan', $jobPath, '--stage', $stage, '--output', $planPath)) { $info.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $null = $process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $peak = 0L
    $timedOut = $false
    while (-not $process.WaitForExit(500)) {
        $process.Refresh()
        $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
        if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            $timedOut = $true
            $process.Kill($true)
            $process.WaitForExit()
            break
        }
    }
    $watch.Stop()
    $stdout.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $outputPath "$stage.stdout.txt")
    $stderr.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $outputPath "$stage.timings.txt")
    $result = [ordered]@{
        stage = $stage
        seconds = $watch.Elapsed.TotalSeconds
        cpu_seconds = $process.TotalProcessorTime.TotalSeconds
        exit_code = $process.ExitCode
        timed_out = $timedOut
        peak_working_set_bytes = $peak
        plan_bytes = $(if (Test-Path -LiteralPath $planPath) { (Get-Item -LiteralPath $planPath).Length } else { $null })
    }
    $results += $result
    $process.Dispose()
    [ordered]@{
        job = $jobPath
        job_sha256 = (Get-FileHash -LiteralPath $jobPath -Algorithm SHA256).Hash
        executable_sha256 = (Get-FileHash -LiteralPath $camPath -Algorithm SHA256).Hash
        runs = $results
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'summary.json')
    $result | ConvertTo-Json -Compress
}
if ($results.Where({ $_.exit_code -ne 0 }).Count) { exit 1 }
