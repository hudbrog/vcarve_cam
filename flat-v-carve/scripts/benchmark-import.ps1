param(
    [string]$Svg = '../real_data/flower_box.svg',
    [int[]]$Copies = @(1, 10, 100),
    [string]$OutputDirectory = 'artifacts/import-scalability',
    [int]$TimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
$benchmarkExe = (Resolve-Path -LiteralPath 'target/release/examples/benchmark_import.exe').Path
$benchmarkSvg = (Resolve-Path -LiteralPath $Svg).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a fresh benchmark output directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$benchmarkResults = @()
foreach ($count in $Copies) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $benchmarkExe
    $info.WorkingDirectory = $PWD.Path
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.ArgumentList.Add($benchmarkSvg)
    $info.ArgumentList.Add([string]$count)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $null = $process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $peak = 0L
    while (!$process.HasExited) {
        $process.Refresh()
        $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
        if ($timer.Elapsed.TotalSeconds -gt $TimeoutSeconds) {
            $process.Kill()
            $process.WaitForExit()
            throw "Import benchmark exceeded $TimeoutSeconds seconds for $count copies."
        }
        Start-Sleep -Milliseconds 25
    }
    $process.WaitForExit()
    $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
    if ($process.ExitCode -ne 0) { throw "Import benchmark failed: $($stderr.Result)" }
    $result = $stdout.Result | ConvertFrom-Json
    $result | Add-Member -NotePropertyName peak_working_set_bytes -NotePropertyValue $peak
    $result | Add-Member -NotePropertyName source_sha256 -NotePropertyValue (Get-FileHash -LiteralPath $benchmarkSvg -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($benchmarkResults.Count -gt 0) {
        $baseline = $benchmarkResults[0]
        $ratio = $result.copies / $baseline.copies
        if ($result.components -ne $baseline.components * $ratio -or $result.sources -ne $baseline.sources * $ratio) { throw 'Replication lost or merged a component.' }
        if ([Math]::Abs($result.area_mm2 - $baseline.area_mm2 * $ratio) -gt $baseline.area_mm2 * $ratio * 0.000001) { throw 'Replication area changed beyond the benchmark comparison tolerance.' }
    }
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory "import-${count}x.json") -Encoding utf8
    $benchmarkResults += $result
    Write-Host "$count copies: $($result.boundary_vertices) vertices, $($result.seconds) s, $peak bytes peak working set"
    $process.Dispose()
}
$benchmarkResults | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'summary.json') -Encoding utf8
