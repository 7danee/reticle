# Builds the release and packs dist/reticle.zip with a "reticle/" folder inside:
#   reticle.exe, readme.txt (guide for friends), LICENSE.txt, notices.txt
#
# Usage: pwsh scripts/package.ps1
# Builds into target/dist, so a running reticle.exe in target/release does not block the build.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

$version = (Select-String -LiteralPath 'Cargo.toml' -Pattern '^version = "(.+)"$' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "reticle v$version"

cargo test --quiet
if ($LASTEXITCODE -ne 0) { throw 'tests failed' }
cargo build --release --target-dir target/dist
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

$dist = Join-Path $root 'dist'
$stage = Join-Path $dist 'stage\reticle'
if (Test-Path -LiteralPath (Join-Path $dist 'stage')) { Remove-Item -LiteralPath (Join-Path $dist 'stage') -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

# Text files with Windows line endings, so every editor shows them correctly.
function Copy-Text($from, $to) {
    $text = (Get-Content -LiteralPath $from -Raw -Encoding utf8) -replace "`r?`n", "`r`n"
    [System.IO.File]::WriteAllText($to, $text, [System.Text.UTF8Encoding]::new($true))
}

Copy-Item -LiteralPath 'target\dist\release\reticle.exe' -Destination $stage
Copy-Text 'packaging\readme.txt' (Join-Path $stage 'readme.txt')
Copy-Text 'LICENSE' (Join-Path $stage 'LICENSE.txt')
Copy-Text 'notices.md' (Join-Path $stage 'notices.txt')

$zip = Join-Path $dist 'reticle.zip'
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
# .NET instead of Compress-Archive, which trips over the [brackets] in this folder path.
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
Remove-Item -LiteralPath (Join-Path $dist 'stage') -Recurse -Force

$item = Get-Item -LiteralPath $zip
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower()
Write-Host ("{0}  {1:N0} KB  sha256 {2}" -f $item.FullName, ($item.Length / 1KB), $hash)
