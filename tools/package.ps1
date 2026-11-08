# Builds a release: the app, the Steam services DLL, the setup (with the app
# inside), a portable zip and SHA256SUMS.txt, all in target/package/dist.
#
#   powershell -ExecutionPolicy Bypass -File tools/package.ps1
#
# Used by .github/workflows/release.yml too.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Packaging SteamInRust $version"

$pkg = Join-Path $root 'target\package'
$stage = Join-Path $pkg 'stage'
$dist = Join-Path $pkg 'dist'
Remove-Item -Recurse -Force $pkg -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage, $dist | Out-Null

cargo build --release -p steam-rs -p steamclient
if ($LASTEXITCODE -ne 0) { throw 'app build failed' }

Copy-Item target\release\steam-rs.exe $stage
Copy-Item target\release\steamclient.dll $stage

# The same DLL for 32-bit games, when a 32-bit MinGW is around
# (i686-w64-mingw32-gcc on PATH, or SIR_MINGW32 = its bin folder).
$gcc32 = if ($env:SIR_MINGW32) { Join-Path $env:SIR_MINGW32 'i686-w64-mingw32-gcc.exe' } else { (Get-Command i686-w64-mingw32-gcc -ErrorAction SilentlyContinue).Source }
if ($gcc32 -and (Test-Path $gcc32)) {
    $env:PATH = (Split-Path $gcc32) + ';' + $env:PATH
    $env:CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER = $gcc32
    rustup target add i686-pc-windows-gnu
    cargo build --release -p steamclient --target i686-pc-windows-gnu
    if ($LASTEXITCODE -ne 0) { throw '32-bit steamclient build failed' }
    Copy-Item target\i686-pc-windows-gnu\release\steamclient.dll (Join-Path $stage 'steamclient32.dll')
} else {
    Write-Warning 'No 32-bit MinGW found: 32-bit games will start without Steam features (no steamclient32.dll).'
}
Copy-Item LICENSE $stage
Copy-Item README.md $stage

$payload = Join-Path $pkg 'payload.zip'
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $payload -CompressionLevel Optimal

$env:SIR_PAYLOAD = $payload
cargo build --release -p installer
if ($LASTEXITCODE -ne 0) { throw 'setup build failed' }
Remove-Item Env:SIR_PAYLOAD

$setup = "SteamInRust-Setup-$version.exe"
$portable = "SteamInRust-$version-portable.zip"
Copy-Item target\release\SteamInRust-Setup.exe (Join-Path $dist $setup)
Copy-Item $payload (Join-Path $dist $portable)

$sums = Get-ChildItem $dist -File | ForEach-Object {
    "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
# LF line endings and no BOM, like sha256sum writes.
[IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS.txt'), (($sums -join "`n") + "`n"))

Get-ChildItem $dist | Format-Table Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
