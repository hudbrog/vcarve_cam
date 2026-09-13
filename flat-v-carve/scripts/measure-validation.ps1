#Requires -Version 7.0
<#
.SYNOPSIS
Measures wall-clock time for each step of the project validation pipeline.

.DESCRIPTION
Runs the same command sequence as .github/workflows/build.yml (the "Build and
test" job) and records, for every step, the wall time, exit code, and captured
output. Results are written incrementally so a long run can be inspected while
it is still going.

The pipeline mirrors CI so its numbers are comparable with CI: the job-level
environment (CARGO_BUILD_TARGET, crt-static RUSTFLAGS) is applied to every
Cargo command, and steps run in the documented order. The GUI steps build the
Rust cam-gui crate for its native and browser targets. The portable executable
is built once by its own Cargo command and then again end to end through
scripts/build-portable.ps1, which should be a cache hit.

.PARAMETER OutputDirectory
Where results.json, summary files, and per-step logs are written. Relative
paths resolve against the flat-v-carve workspace root.

.PARAMETER Label
Free-form label stored in the results (for example "ci-profile", "rerun").

.PARAMETER Offline
Pass --offline to the portable build script.

.PARAMETER PortableOutputDirectory
Where scripts/build-portable.ps1 writes cam.exe, and the executable that the
live integration step tests. Defaults to artifacts/portable.

.PARAMETER Only
Run only the listed step ids. Use -List to print the ids.

.PARAMETER RustFlagsProfile
'ci' applies the crt-static RUSTFLAGS that build.yml sets for the whole job.
'dev' leaves RUSTFLAGS untouched, matching an ordinary local Cargo build.

.EXAMPLE
./scripts/measure-validation.ps1 -Label ci-profile

.EXAMPLE
./scripts/measure-validation.ps1 -Only rust-fmt,gui-native-build -OutputDirectory artifacts/validation-timing/quick
#>
[CmdletBinding()]
param(
    [string]$OutputDirectory = 'artifacts/validation-timing',
    [string]$Label = '',
    [switch]$Offline,
    [string]$PortableOutputDirectory = 'artifacts/portable',
    [string[]]$Only = @(),
    [ValidateSet('ci', 'dev')][string]$RustFlagsProfile = 'ci',
    [switch]$ExportCargoTimings,
    [switch]$List
)

$ErrorActionPreference = 'Stop'
$camWorkspace = Split-Path -Parent $PSScriptRoot
$camOutput = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $camWorkspace $OutputDirectory))
}
$camLogDirectory = Join-Path $camOutput 'steps'
$camPortableOutput = if ([System.IO.Path]::IsPathRooted($PortableOutputDirectory)) {
    [System.IO.Path]::GetFullPath($PortableOutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $camWorkspace $PortableOutputDirectory))
}

# Accept both -Only a,b,c (interactive parsing) and -Only a,b,c (pwsh -File,
# which hands the whole list over as one string).
$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne '' })

function Resolve-CamTool {
    param([Parameter(Mandatory)][string]$Name)
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $command) { throw "Required tool '$Name' was not found on PATH." }
    return $command.Source
}

$camCargo = Resolve-CamTool -Name 'cargo'
$camNode = Resolve-CamTool -Name 'node'

# One entry per command. CiStep names the build.yml step each measurement maps to.
$camSteps = @(
    [pscustomobject]@{
        Id = 'rust-fmt'; Category = 'rust-format'; Kind = 'exe'; Tool = $camCargo
        Args = @('fmt', '--all', '--', '--check'); WorkDir = $camWorkspace
        CiStep = 'Check Rust formatting'
        Description = 'Formatting drift check across the workspace'
    }
    [pscustomobject]@{
        Id = 'gui-native-build'; Category = 'gui-build'; Kind = 'script'; Tool = (Join-Path $camWorkspace 'scripts/build-gui.ps1')
        Params = @{}; WorkDir = $camWorkspace
        CiStep = 'Build native GUI'
        Description = 'scripts/build-gui.ps1 (cargo build --workspace --release --bin cam-gui)'
    }
    [pscustomobject]@{
        Id = 'gui-web-build'; Category = 'wasm-build'; Kind = 'script'; Tool = (Join-Path $camWorkspace 'scripts/build-gui.ps1')
        Params = @{ Target = 'web' }; WorkDir = $camWorkspace
        CiStep = 'Build browser GUI'
        Description = 'scripts/build-gui.ps1 -Target web (wasm-pack build cam-gui)'
    }
    [pscustomobject]@{
        Id = 'portable-cargo-build'; Category = 'rust-release-build'; Kind = 'exe'; Tool = $camCargo
        Args = @('build', '--release', '--locked', '--workspace', '--bin', 'cam', '--target', 'x86_64-pc-windows-msvc'); WorkDir = $camWorkspace
        CiStep = 'Build portable executable (cargo build)'
        Description = 'Release build of the command-line cam.exe and static CRT'
    }
    [pscustomobject]@{
        Id = 'portable-script'; Category = 'portable-package'; Kind = 'script'; Tool = (Join-Path $camWorkspace 'scripts/build-portable.ps1')
        Params = @{ OutputDirectory = $camPortableOutput; Offline = [bool]$Offline }; WorkDir = $camWorkspace
        CiStep = 'Build portable executable'
        Description = 'scripts/build-portable.ps1 end-to-end (expected cache hit after the expanded step)'
    }
    [pscustomobject]@{
        Id = 'rust-clippy'; Category = 'rust-lint'; Kind = 'exe'; Tool = $camCargo
        Args = @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'); WorkDir = $camWorkspace
        CiStep = 'Check Rust with Clippy'
        Description = 'Workspace clippy with all targets'
    }
    [pscustomobject]@{
        Id = 'rust-test-compile'; Category = 'rust-test-build'; Kind = 'exe'; Tool = $camCargo
        Args = @('test', '--workspace', '--release', '--locked', '--no-run') +
            $(if ($ExportCargoTimings) { @('--timings=html,json') } else { @() }); WorkDir = $camWorkspace
        CiStep = 'Run Rust tests (compile half)'
        Description = 'Release build of every test binary, no tests executed'
    }
    [pscustomobject]@{
        Id = 'rust-test-run'; Category = 'rust-test-run'; Kind = 'script'; Tool = (Join-Path $camWorkspace 'scripts/run-tests-parallel.ps1')
        Params = @{ OutputDirectory = (Join-Path $camOutput 'test-parallel') }; WorkDir = $camWorkspace
        CiStep = 'Run Rust tests'
        Description = 'scripts/run-tests-parallel.ps1 worker-pool execution of the compiled test binaries'
    }
)

if ($List) {
    $camSteps | ForEach-Object { '{0,-22} {1,-20} {2}' -f $_.Id, $_.Category, $_.CiStep }
    return
}

if ($RustFlagsProfile -eq 'ci' -and ($env:RUSTFLAGS -or $env:CARGO_ENCODED_RUSTFLAGS)) {
    throw 'Unset RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS before measuring the ci profile.'
}

New-Item -ItemType Directory -Path $camLogDirectory -Force | Out-Null
$camResultsPath = Join-Path $camOutput 'results.json'

# Job-level environment from build.yml, applied to every step.
$camJobEnvironment = [ordered]@{ CARGO_TERM_COLOR = 'always' }
if ($RustFlagsProfile -eq 'ci') {
    $camJobEnvironment['CARGO_BUILD_TARGET'] = 'x86_64-pc-windows-msvc'
    $camJobEnvironment['CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS'] = '-C target-feature=+crt-static'
}

$camGitCommit = (& git -C $camWorkspace rev-parse HEAD 2>$null)
$camGitDirty = @(& git -C $camWorkspace status --porcelain 2>$null).Count

$camEnvironment = [ordered]@{
    host = [System.Environment]::MachineName
    os = "$([System.Environment]::OSVersion.VersionString)"
    cpu = $env:PROCESSOR_IDENTIFIER
    logicalProcessors = [System.Environment]::ProcessorCount
    gitCommit = "$camGitCommit"
    gitDirtyFiles = $camGitDirty
    rustFlagsProfile = $RustFlagsProfile
    offline = [bool]$Offline
    cargo = (& $camCargo --version)
    node = (& $camNode --version)
    rustc = (& (Join-Path (Split-Path $camCargo -Parent) 'rustc') --version 2>$null)
    wasmPack = (& (Resolve-CamTool -Name 'wasm-pack') --version 2>$null)
    jobEnvironment = $camJobEnvironment
}

# Hardware detail is best-effort: CIM/WMI may be unavailable inside a sandbox.
try {
    $camCpuInfo = Get-CimInstance -ClassName Win32_Processor -ErrorAction Stop | Select-Object -First 1
    $camOsInfo = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
    $camEnvironment.cpu = $camCpuInfo.Name
    $camEnvironment.physicalCores = $camCpuInfo.NumberOfCores
    $camEnvironment.logicalProcessors = $camCpuInfo.NumberOfLogicalProcessors
    $camEnvironment.os = "$($camOsInfo.Caption) $($camOsInfo.Version)"
    $camEnvironment.memoryGB = [math]::Round($camOsInfo.TotalVisibleMemorySize / 1MB, 1)
} catch {
    $camEnvironment.hardwareDetail = "unavailable: $($_.Exception.Message)"
}

$camSelected = if ($Only.Count -gt 0) { $camSteps | Where-Object { $_.Id -in $Only } } else { $camSteps }
if ($Only.Count -gt 0 -and $camSelected.Count -ne $Only.Count) {
    $camUnknown = $Only | Where-Object { $_ -notin $camSteps.Id }
    throw "Unknown step id(s): $($camUnknown -join ', '). Use -List to see valid ids."
}

$camRun = [ordered]@{
    schema = 1
    label = $Label
    startedUtc = [DateTime]::UtcNow.ToString('o')
    finishedUtc = $null
    totalSeconds = $null
    environment = $camEnvironment
    stepOrder = @($camSteps.Id)
    steps = @()
}
($camRun | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $camResultsPath -Encoding utf8NoBOM

function Save-CamResults {
    ($camRun | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $camResultsPath -Encoding utf8NoBOM
}

# Save and restore every environment variable the run touches.
$camSavedEnv = @{}
foreach ($camName in $camJobEnvironment.Keys) {
    $camSavedEnv[$camName] = [System.Environment]::GetEnvironmentVariable($camName)
}
$camScriptEnvironment = @{}

try {
    foreach ($camName in $camJobEnvironment.Keys) {
        [System.Environment]::SetEnvironmentVariable($camName, $camJobEnvironment[$camName])
    }

    $camIndex = 0
    $camRunStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    foreach ($camStep in $camSelected) {
        $camIndex++
        Write-Host ("[{0}/{1}] {2} - {3}" -f $camIndex, $camSelected.Count, $camStep.Id, $camStep.Description)

        foreach ($camName in @($camScriptEnvironment.Keys)) {
            [System.Environment]::SetEnvironmentVariable($camName, $null)
            $camScriptEnvironment.Remove($camName)
        }
        if ($camStep.PSObject.Properties.Name -contains 'Env') {
            foreach ($camName in $camStep.Env.Keys) {
                $camScriptEnvironment[$camName] = $camStep.Env[$camName]
                [System.Environment]::SetEnvironmentVariable($camName, $camStep.Env[$camName])
            }
        }

        $camLogPath = Join-Path $camLogDirectory ("{0}.log" -f $camStep.Id)
        $camOutputText = @()
        $camExitCode = 0
        $camStartedUtc = [DateTime]::UtcNow.ToString('o')
        $camStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        Push-Location -LiteralPath $camStep.WorkDir
        try {
            $camArgs = @($camStep.Args)
            if ($camStep.Kind -eq 'exe') {
                $camOutputText = & $camStep.Tool @camArgs 2>&1
                $camExitCode = $LASTEXITCODE
                if ($null -eq $camExitCode) { $camExitCode = 0 }
            } else {
                $camParams = $camStep.Params
                $global:LASTEXITCODE = 0
                # *>&1 also captures Write-Host (information stream) into the log.
                & $camStep.Tool @camParams *>&1 | ForEach-Object { $camOutputText += $_ }
                $camExitCode = if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { $LASTEXITCODE } else { 0 }
            }
        } catch {
            $camOutputText += "HARNESS ERROR: $($_.Exception.Message)"
            $camExitCode = 1
        } finally {
            $camStopwatch.Stop()
            Pop-Location
        }
        $camFinishedUtc = [DateTime]::UtcNow.ToString('o')

        # -Value (not a pipeline) so a step that prints nothing still gets a log file.
        $camLogText = (($camOutputText | ForEach-Object { "$_" }) -join [Environment]::NewLine)
        Set-Content -LiteralPath $camLogPath -Value $camLogText -Encoding utf8NoBOM

        # Count from the written log: stream redirection turns some cargo lines
        # into error records, and CARGO_TERM_COLOR adds escape codes, so match on
        # the file text with the colour codes removed.
        $camPlainLog = $camLogText -replace "$([char]27)\[[0-9;]*m", ''
        $camCompiling = ([regex]::Matches(
                $camPlainLog,
                '(?m)^\s+(?:Compiling|Checking)\s+\S+\s+v')).Count
        $camEntry = [ordered]@{
            id = $camStep.Id
            category = $camStep.Category
            ciStep = $camStep.CiStep
            description = $camStep.Description
            command = if ($camStep.Kind -eq 'exe') {
                "$([System.IO.Path]::GetFileName($camStep.Tool)) $($camStep.Args -join ' ')"
            } else {
                $camRendered = $camStep.Params.GetEnumerator() | ForEach-Object {
                    if ($_.Value -eq $true) { "-$($_.Key)" }
                    elseif ($_.Value -eq $false) { $null }
                    else { "-$($_.Key) $($_.Value)" }
                }
                "$([System.IO.Path]::GetFileName($camStep.Tool)) $(@($camRendered) -join ' ')"
            }
            workDir = $camStep.WorkDir
            startUtc = $camStartedUtc
            endUtc = $camFinishedUtc
            seconds = [math]::Round($camStopwatch.Elapsed.TotalSeconds, 3)
            exitCode = $camExitCode
            status = if ($camExitCode -eq 0) { 'ok' } else { 'fail' }
            rebuiltCrates = $camCompiling
            log = $camLogPath
        }
        $camRun.steps = @($camRun.steps) + @([pscustomobject]$camEntry)
        Save-CamResults

        Write-Host ("          {0:N1}s  exit={1}  rebuilt={2}" -f $camEntry.seconds, $camExitCode, $camCompiling)
    }
    $camRunStopwatch.Stop()
    $camRun.totalSeconds = [math]::Round($camRunStopwatch.Elapsed.TotalSeconds, 3)
} finally {
    $camRun.finishedUtc = [DateTime]::UtcNow.ToString('o')
    if ($null -eq $camRun.totalSeconds) {
        $camRun.totalSeconds = [math]::Round(([DateTime]::Parse($camRun.finishedUtc) - [DateTime]::Parse($camRun.startedUtc)).TotalSeconds, 3)
    }
    Save-CamResults
    foreach ($camName in $camScriptEnvironment.Keys) {
        [System.Environment]::SetEnvironmentVariable($camName, $null)
    }
    foreach ($camName in $camSavedEnv.Keys) {
        [System.Environment]::SetEnvironmentVariable($camName, $camSavedEnv[$camName])
    }
}

$camFailed = @($camRun.steps | Where-Object status -eq 'fail')
Write-Host ''
Write-Host ("Total: {0:N1}s over {1} steps ({2} failed)" -f $camRun.totalSeconds, @($camRun.steps).Count, $camFailed.Count)
Write-Host "Results: $camResultsPath"
if ($camFailed.Count -gt 0) {
    Write-Host ("Failed steps: {0}" -f (($camFailed | ForEach-Object id) -join ', '))
}
