# DuckCM 发布更新脚本：改版本号 → 构建 → 提交 bin\exDuckCM.exe + version.json → tag → push
# 用法：在 duckcm-rust 目录执行  .\publish_update.ps1 2.4.0 "更新说明"
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Notes
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# 1. Cargo.toml 版本号
((Get-Content Cargo.toml -Raw -Encoding utf8) -replace '(?m)^version = "[^"]+"', "version = `"$Version`"") |
    Set-Content Cargo.toml -NoNewline -Encoding utf8

# 2. 构建 release
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'release build failed' }

# 3. 发布件入库
New-Item -ItemType Directory -Force bin | Out-Null
Copy-Item target\release\DuckCM.exe bin\DuckCM.exe -Force
@{ version = $Version; notes = $Notes; file = 'bin/DuckCM.exe' } |
    ConvertTo-Json | Set-Content version.json -Encoding utf8

# 4. 提交 + 打 tag + 推送（CDN 以 tag 取件）
git add Cargo.toml version.json bin/DuckCM.exe
git commit -m "release v$Version"
git tag "v$Version"
git push origin main "v$Version"

Write-Host "v$Version 已推送。jsDelivr CDN 缓存数分钟内刷新（@latest），" -NoNewline
Write-Host "更新源: https://cdn.jsdelivr.net/gh/hqzzzz/DuckCM@latest/version.json"
