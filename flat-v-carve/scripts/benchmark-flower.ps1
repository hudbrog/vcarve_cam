param(
    [string]$Job = '../real_data/flower_box-svg.job-real.json',
    [string]$OutputDirectory = 'artifacts/flower-performance',
    [string]$Cam = '',
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 300,
    [ValidateRange(1, 100)][int]$Repetitions = 1,
    [ValidateRange(0, 86400)][double]$MaxSeconds = 0,
    [ValidateNotNullOrEmpty()][ValidateSet('endmill', 'combined')][string[]]$Stages = @('endmill', 'combined')
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
$jobPath = (Resolve-Path -LiteralPath $Job).Path
$camPath = if ($Cam) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Cam) } else { Join-Path $workspace 'target/release/cam.exe' }
if (-not (Test-Path -LiteralPath $camPath)) { throw 'Build first: cargo build --release --locked -p cam-app' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new output directory.' }
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$jobHash = (Get-FileHash -LiteralPath $jobPath -Algorithm SHA256).Hash
$executableHash = (Get-FileHash -LiteralPath $camPath -Algorithm SHA256).Hash
$results = @()
$runsToExecute = foreach ($repetition in 1..$Repetitions) {
    foreach ($stage in $Stages) { [PSCustomObject]@{ stage = $stage; repetition = $repetition } }
}
foreach ($run in $runsToExecute) {
    $stage = $run.stage
    $stem = if ($Repetitions -eq 1) { $stage } else { "$stage.$($run.repetition)" }
    $planPath = Join-Path $outputPath "$stem.plan.json"
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
    $stdout.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $outputPath "$stem.stdout.txt")
    $log = $stderr.GetAwaiter().GetResult()
    $log | Set-Content -LiteralPath (Join-Path $outputPath "$stem.timings.txt")
    # Exit zero also accepts Empty in the CLI; a flower benchmark must Complete.
    $statusPattern = if ($stage -eq 'combined') { '(?m)^Combined M4 stage: (\w+);' } else { '(?m)^Endmill stage: (\w+);' }
    $statusMatch = [regex]::Match($log, $statusPattern)
    $status = if ($statusMatch.Success) { $statusMatch.Groups[1].Value } else { $null }
    $planExists = Test-Path -LiteralPath $planPath
    $complete = $process.ExitCode -eq 0 -and -not $timedOut -and $status -eq 'Complete' -and $planExists
    $withinBudget = $MaxSeconds -eq 0 -or $watch.Elapsed.TotalSeconds -lt $MaxSeconds
    $result = [ordered]@{
        stage = $stage
        repetition = $run.repetition
        seconds = $watch.Elapsed.TotalSeconds
        cpu_seconds = $process.TotalProcessorTime.TotalSeconds
        exit_code = $process.ExitCode
        timed_out = $timedOut
        status = $status
        passed = $complete -and $withinBudget
        peak_working_set_bytes = $peak
        plan_bytes = $(if ($planExists) { (Get-Item -LiteralPath $planPath).Length } else { $null })
        plan_sha256 = $(if ($planExists) { (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash } else { $null })
    }
    $results += $result
    $process.Dispose()
    [ordered]@{
        job = $jobPath
        job_sha256 = $jobHash
        executable_sha256 = $executableHash
        max_seconds = $(if ($MaxSeconds -gt 0) { $MaxSeconds } else { $null })
        logical_processors = [Environment]::ProcessorCount
        runs = $results
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'summary.json')
    $result | ConvertTo-Json -Compress
}
if ($results.Where({ -not $_.passed }).Count) { exit 1 }
exit 0
