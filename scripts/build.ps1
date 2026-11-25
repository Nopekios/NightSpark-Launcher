$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "NightSpark community build (no official signing key, watermark uses the public salt)"

if (-not (Test-Path (Join-Path $root "node_modules"))) {
    npm install
    if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
}

npm run tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$wm = Get-ChildItem -Path (Join-Path $root "src-tauri") -Recurse -Filter "watermark.rs" -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like "*\out\*" } |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($wm) {
    $tagLine = Select-String -Path $wm.FullName -Pattern 'pub const TAG: &str = ' | Select-Object -First 1
    $offLine = Select-String -Path $wm.FullName -Pattern 'pub const OFFICIAL: bool = ' | Select-Object -First 1
    Write-Host ""
    if ($tagLine) { Write-Host $tagLine.Line.Trim() }
    if ($offLine) { Write-Host $offLine.Line.Trim() }
    Write-Host "OFFICIAL=false builds are not accepted as official releases"
}

$bundle = $null
foreach ($rel in @("src-tauri\.ctarget\release\bundle", "src-tauri\target\release\bundle")) {
    $candidate = Join-Path $root $rel
    if (Test-Path $candidate) { $bundle = $candidate; break }
}
if ($bundle) {
    Write-Host "bundle: $bundle"
    Get-ChildItem -Path $bundle -Recurse -Include *.exe | Select-Object FullName, Length | Format-Table -AutoSize
} else {
    Write-Host "bundle directory not found, check the build output above"
}
