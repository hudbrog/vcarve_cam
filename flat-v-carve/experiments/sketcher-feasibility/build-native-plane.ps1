$ErrorActionPreference='Stop'
$experiment=$PSScriptRoot
$workspace=[IO.Path]::GetFullPath((Join-Path $experiment '../..'))
$upstream=Join-Path $workspace 'artifacts/sketcher-feasibility/upstream'
$plane=Join-Path $upstream 'planegcs-9d26baef0b240fe0eef4186d4e5e5e66ec3620b7/planegcs'
$output=Join-Path $workspace 'artifacts/sketcher-feasibility/native-plane'
New-Item -ItemType Directory -Force $output | Out-Null
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs=& $vswhere -latest -products '*' -property installationPath
$vc=(Get-ChildItem (Join-Path $vs 'VC/Tools/MSVC') -Directory | Sort-Object Name -Descending | Select-Object -First 1).FullName
$sdk=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
$sdkVersion=(Get-ChildItem (Join-Path $sdk 'Include') -Directory | Sort-Object Name -Descending | Select-Object -First 1).Name
$env:PATH="$vc/bin/Hostx64/x64;$env:PATH"
$includes=@("$vc/include","$sdk/Include/$sdkVersion/ucrt","$sdk/Include/$sdkVersion/shared","$sdk/Include/$sdkVersion/um") | ForEach-Object {"/I$_"}
$sources=@('GCS.cpp','Geo.cpp','Constraints.cpp','SubSystem.cpp','qp_eq.cpp') | ForEach-Object {Join-Path $plane $_}
Push-Location $output
try {
    & "$vc/bin/Hostx64/x64/cl.exe" /nologo /O2 /EHsc /std:c++20 /MD /DNDEBUG /D_USE_MATH_DEFINES /bigobj /W0 /MP4 @includes "/I$experiment/native" "/I$plane" "/I$plane/headers" "/I$upstream/eigen-3.4.0" "/I$upstream/boost_1_86_0" "/I$upstream" @sources "$experiment/native/main.cpp" /Fe:plane-probe.exe /link "/LIBPATH:$vc/lib/x64" "/LIBPATH:$sdk/Lib/$sdkVersion/ucrt/x64" "/LIBPATH:$sdk/Lib/$sdkVersion/um/x64"
    if($LASTEXITCODE -ne 0){throw 'PlaneGCS compilation failed'}
} finally {Pop-Location}
