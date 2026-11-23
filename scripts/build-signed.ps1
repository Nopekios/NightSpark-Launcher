# NightSpark 签名构建脚本
# 用 minisign 私钥对构建产物签名，生成可被官方客户端校验的更新包与 latest.json
# 私钥保存在仓库之外，绝不入库、绝不随包分发

param(
    [string]$KeyPath = "$env:USERPROFILE\.nightspark-signing\nightspark.key",
    [string]$KeyPassword = ""
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $KeyPath)) {
    throw "私钥不存在：$KeyPath"
}

$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$env:TAURI_SIGNING_PRIVATE_KEY = $KeyPath
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $KeyPassword

Write-Host "==> 构建并签名 NightSpark" -ForegroundColor Cyan
npm run tauri build
if ($LASTEXITCODE -ne 0) { throw "构建失败" }

# 汇总签名产物，生成 latest.json（updater 端点读取的清单）
$bundleDir = Join-Path $root "src-tauri\target\release\bundle\nsis"
$conf = Get-Content (Join-Path $root "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$version = $conf.version
$repo = "Nopekios/NightSpark-Launcher"

$sigFiles = Get-ChildItem $bundleDir -Filter *.sig -ErrorAction SilentlyContinue
if (-not $sigFiles) {
    throw "未找到签名产物（.sig），请确认 tauri.conf.json 里 createUpdaterArtifacts 已开启"
}

$platforms = [ordered]@{}
foreach ($sig in $sigFiles) {
    $target = $sig.Name -replace '\.sig$', ''
    $signature = (Get-Content $sig.FullName -Raw).Trim()
    $platforms["windows-x86_64"] = [ordered]@{
        signature = $signature
        url       = "https://github.com/$repo/releases/download/v$version/$target"
    }
    Write-Host "==> 已签名：$target" -ForegroundColor Green
}

$latest = [ordered]@{
    version   = $version
    notes     = ""
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = $platforms
}

$latestPath = Join-Path $bundleDir "latest.json"
$latest | ConvertTo-Json -Depth 6 | Set-Content $latestPath -Encoding UTF8
Write-Host "==> 更新清单：$latestPath" -ForegroundColor Green
Write-Host "==> 发布时把安装包、.sig 与 latest.json 一并上传到 GitHub Releases" -ForegroundColor Yellow
