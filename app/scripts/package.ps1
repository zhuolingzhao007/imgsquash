#Requires -Version 5.1
# 打包 ImgSquash（Windows）。对应 scripts/package.sh。
#
# 产物在 .nsh\out：一个含 imgsquash.exe 的目录，和同名 zip。
#
# nsh 生成的 release profile 不带 strip，符号表要占几 MB，打包时传
# CARGO_PROFILE_RELEASE_STRIP=symbols 让 cargo 剥掉。
# .nsh\rust\Cargo.toml 每次打包都会被 nsh 重新生成，手改无效，profile 只能走环境变量。
#
# 用法：powershell -ExecutionPolicy Bypass -File scripts\package.ps1

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$name = 'imgsquash'

if (-not (Get-Command nsh -ErrorAction SilentlyContinue)) {
    throw '找不到 nsh。先装：irm https://nsh.dev/install.ps1 | iex'
}

# 确保 nsh.json 的 name 一致。只在真的不一致时才写回：PowerShell 5.1 的
# Set-Content -Encoding utf8 会带 BOM，nsh 读不了，所以用 .NET 写不带 BOM 的 UTF-8。
$manifestPath = Join-Path (Get-Location) 'nsh.json'
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
if ($manifest.name -ne $name) {
    $manifest.name = $name
    $json = $manifest | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText($manifestPath, $json, (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "nsh.json 的 name 已改为 $name"
}

# 清掉上一次的产物，免得把旧的 exe / zip 当成这次的。
$outDir = Join-Path (Get-Location) '.nsh\out'
if (Test-Path $outDir) {
    Get-ChildItem $outDir | Remove-Item -Recurse -Force
}

$env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'
try {
    nsh package --target windows --name $name --icon assets/icon.png
} finally {
    Remove-Item Env:\CARGO_PROFILE_RELEASE_STRIP -ErrorAction SilentlyContinue
}

# 产物名以 nsh 实际给的为准，别硬编码目录名。
$exe = Get-ChildItem $outDir -Filter "$name.exe" -Recurse -File | Select-Object -First 1
if (-not $exe) {
    throw "没找到 $name.exe，看看 $outDir 里是什么"
}
$zip = Get-ChildItem $outDir -Filter "$name.zip" -File | Select-Object -First 1

Write-Host ''
Write-Host '打包完成：'
Write-Host ("  exe  {0}  ({1:N1} MB)" -f $exe.FullName, ($exe.Length / 1MB))
if ($zip) {
    Write-Host ("  zip  {0}  ({1:N1} MB)" -f $zip.FullName, ($zip.Length / 1MB))
}
