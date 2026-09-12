#Requires -Version 7.0
<#
.SYNOPSIS
Runs the workspace's already-built Rust test binaries in parallel.

.DESCRIPTION
`cargo test --workspace` compiles every test binary and then runs them one after
another, so wall time is the sum of all binaries even when most of them are
short. This script discovers the built binaries from Cargo's JSON messages and
runs them with a worker pool instead, which is what cargo-nextest does.

The workspace has 68 test binaries; the longest single binary is about 12s while
the sum is about 50s, so overlapping them cuts the test step roughly threefold on
a 16-thread machine. Pass -SkipBuild in CI where the compile step already ran
`cargo test --no-run`.

Test binaries run with their package directory as the working directory and with
Cargo's default environment, so results match `cargo test`.

.PARAMETER Workers
How many test binaries run at once. Defaults to the logical processor count,
capped at 8 (so 4-core CI runners run 4 binaries at once).

.PARAMETER Threads
Threads per test binary (libtest --test-threads). Defaults to the remaining
logical processors, so Workers x Threads stays near the core count.

Measured on a 16-thread machine: 1 worker (the plain `cargo test` shape) needs
~45-54s, while 8 workers x 2 threads needs ~15-20s. On a 4-core budget the
comparison is 70s for 1 worker x 4 threads against 50s for 4 workers x 1 thread.

.PARAMETER SkipBuild
Reuse the results of a previous `cargo test --no-run` instead of building.

.PARAMETER FailFast
Stop scheduling new binaries after the first failure.

.EXAMPLE
cargo test --workspace --release --locked --no-run
./scripts/run-tests-parallel.ps1 -SkipBuild
#>
[CmdletBinding()]
param(
    [int]$Workers = 0,
    [int]$Threads = 0,
    [switch]$SkipBuild,
    [switch]$FailFast,
    [string]$OutputDirectory = 'artifacts/test-parallel',
    [string]$ManifestPath = 'Cargo.toml'
)
$ErrorActionPreference = 'Stop'
$camWorkspace = Split-Path -Parent $PSScriptRoot
$camOutput = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $camWorkspace $OutputDirectory))
}
$camCargo = (Get-Command cargo -ErrorAction Stop).Source

$camCores = [System.Environment]::ProcessorCount
if ($Workers -le 0) { $Workers = [math]::Max(2, [math]::Min(8, $camCores)) }
if ($Threads -le 0) { $Threads = [math]::Max(1, [math]::Floor($camCores / $Workers)) }

New-Item -ItemType Directory -Path $camOutput -Force | Out-Null
$camLogDirectory = Join-Path $camOutput ('w{0}-t{1}' -f $Workers, $Threads)
if (Test-Path $camLogDirectory) { Remove-Item -LiteralPath $camLogDirectory -Recurse -Force }
New-Item -ItemType Directory -Path $camLogDirectory -Force | Out-Null

Push-Location -LiteralPath $camWorkspace
try {
    if (-not $SkipBuild) {
        Write-Host 'Compiling test binaries...'
        $camBuildLog = Join-Path $camOutput 'build.jsonl'
        & $camCargo test --manifest-path $ManifestPath --workspace --release --locked --no-run --message-format=json 2>$null |
            Set-Content -LiteralPath $camBuildLog
        if ($LASTEXITCODE -ne 0) {
            Get-Content -LiteralPath $camBuildLog | ForEach-Object {
                $camMessage = $_ | ConvertFrom-Json
                if ($camMessage.reason -eq 'compiler-message' -and $camMessage.message.level -eq 'error') {
                    Write-Host $camMessage.message.rendered
                }
            }
            throw 'cargo test --no-run failed.'
        }
    } else {
        $camBuildLog = Join-Path $camOutput 'build.jsonl'
        if (-not (Test-Path $camBuildLog)) {
            throw "$camBuildLog is missing. Run without -SkipBuild, or point -OutputDirectory at a previous run."
        }
    }

    $camTests = Get-Content -LiteralPath $camBuildLog | ForEach-Object {
        $camMessage = $_ | ConvertFrom-Json
        if ($camMessage.reason -ne 'compiler-artifact') { return }
        if (-not $camMessage.executable) { return }
        if (-not $camMessage.profile.test) { return }
        [pscustomobject]@{
            Name = "$(($camMessage.package_id -split '#')[-1] -replace '@', ' v')::$($camMessage.target.name)"
            Exe = $camMessage.executable
            Directory = Split-Path $camMessage.manifest_path -Parent
        }
    } | Sort-Object Exe -Unique
    if ($camTests.Count -eq 0) { throw 'No test binaries were found in the Cargo output.' }

    Write-Host ("Running {0} test binaries: {1} at a time, {2} thread(s) each ({3} logical processors)." -f $camTests.Count, $Workers, $Threads, $camCores)
    $camQueue = [System.Collections.Queue]::new()
    foreach ($camTest in $camTests) { $camQueue.Enqueue($camTest) }
    $camRunning = [System.Collections.Generic.List[object]]::new()
    $camFinished = [System.Collections.Generic.List[object]]::new()
    $camWatch = [System.Diagnostics.Stopwatch]::StartNew()
    $camCancelled = $false

    while ($camQueue.Count -gt 0 -or $camRunning.Count -gt 0) {
        while (-not $camCancelled -and $camRunning.Count -lt $Workers -and $camQueue.Count -gt 0) {
            $camTest = $camQueue.Dequeue()
            $camIndex = $camFinished.Count + $camRunning.Count
            $camSafe = ($camTest.Name -replace '[^A-Za-z0-9_.-]', '_')
            $camProcess = Start-Process -FilePath $camTest.Exe -ArgumentList @('--test-threads', "$Threads") `
                -WorkingDirectory $camTest.Directory -PassThru -NoNewWindow `
                -RedirectStandardOutput (Join-Path $camLogDirectory "$camIndex-$camSafe.out") `
                -RedirectStandardError (Join-Path $camLogDirectory "$camIndex-$camSafe.err")
            $camRunning.Add([pscustomobject]@{ Process = $camProcess; Test = $camTest; Start = $camWatch.Elapsed.TotalSeconds; Index = $camIndex })
        }
        Start-Sleep -Milliseconds 50
        for ($camIndex = $camRunning.Count - 1; $camIndex -ge 0; $camIndex--) {
            $camEntry = $camRunning[$camIndex]
            if ($camEntry.Process.HasExited) {
                $camFinished.Add([pscustomobject]@{
                    Name = $camEntry.Test.Name
                    Seconds = [math]::Round($camWatch.Elapsed.TotalSeconds - $camEntry.Start, 3)
                    ExitCode = $camEntry.Process.ExitCode
                    Index = $camEntry.Index
                })
                if ($camEntry.Process.ExitCode -ne 0 -and $FailFast) { $camCancelled = $true }
                $camRunning.RemoveAt($camIndex)
            }
        }
    }
    $camWatch.Stop()

    # Aggregate the per-binary libtest summaries so the output matches `cargo test`.
    $camPassed = 0; $camFailed = 0; $camIgnored = 0; $camMeasured = 0; $camFiltered = 0
    foreach ($camEntry in $camFinished) {
        $camSafe = ($camEntry.Name -replace '[^A-Za-z0-9_.-]', '_')
        $camLogPath = Join-Path $camLogDirectory "$($camEntry.Index)-$camSafe.out"
        if (-not (Test-Path $camLogPath)) { continue }
        $camText = Get-Content -LiteralPath $camLogPath -Raw
        foreach ($camMatch in [regex]::Matches($camText, 'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out')) {
            $camPassed += [int]$camMatch.Groups[1].Value
            $camFailed += [int]$camMatch.Groups[2].Value
            $camIgnored += [int]$camMatch.Groups[3].Value
            $camMeasured += [int]$camMatch.Groups[4].Value
            $camFiltered += [int]$camMatch.Groups[5].Value
        }
    }

    $camBroken = @($camFinished | Where-Object { $_.ExitCode -ne 0 })
    $camSkipped = $camTests.Count - $camFinished.Count
    $camStatus = if ($camBroken.Count -eq 0 -and $camSkipped -eq 0) { 'ok' } else { 'FAILED' }
    # Invariant culture so the summary parses the same way on every locale.
    $camWall = $camWatch.Elapsed.TotalSeconds.ToString('0.00', [System.Globalization.CultureInfo]::InvariantCulture)
    if ($camStatus -eq 'ok') {
        Write-Host ("test result: ok. {0} passed; {1} failed; {2} ignored; {3} measured; {4} filtered out; finished in {5}s" -f `
                $camPassed, $camFailed, $camIgnored, $camMeasured, $camFiltered, $camWall)
    } else {
        Write-Host ("test result: FAILED. {0} passed; {1} failed; {2} ignored; {3} binaries failed; {4} not run; finished in {5}s" -f `
                $camPassed, $camFailed, $camIgnored, $camBroken.Count, $camSkipped, $camWall)
        foreach ($camEntry in $camBroken) {
            Write-Host ("  failing binary: {0} (exit {1})" -f $camEntry.Name, $camEntry.ExitCode)
        }
    }
    $camSum = [double]($camFinished | Measure-Object Seconds -Sum).Sum
    $camLongest = [double]($camFinished | Measure-Object Seconds -Maximum).Maximum
    $camInvariant = [System.Globalization.CultureInfo]::InvariantCulture
    Write-Host ("Ran {0}/{1} binaries in {2}s (sum {3}s, longest {4}s)." -f `
            $camFinished.Count, $camTests.Count,
        $camWatch.Elapsed.TotalSeconds.ToString('0.0', $camInvariant), $camSum.ToString('0.0', $camInvariant), $camLongest.ToString('0.0', $camInvariant))
    $camFinished | Sort-Object Seconds -Descending | Export-Csv -LiteralPath (Join-Path $camOutput 'binaries.csv') -NoTypeInformation

    if ($camStatus -ne 'ok') { exit 1 }
} finally {
    Pop-Location
}
