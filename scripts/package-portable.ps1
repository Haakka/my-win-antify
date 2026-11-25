$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
[System.Environment]::CurrentDirectory = $Root

# Keep rustc's self-contained MinGW linker cleanly ahead without w64devkit\bin shadowing -lgcc_eh.
$pathPrefix = @("$env:USERPROFILE\.cargo\bin")
if (Test-Path "$env:USERPROFILE\.local\mingw-tools") {
    $pathPrefix += "$env:USERPROFILE\.local\mingw-tools"
}
$filteredPath = $env:PATH -split ';' | Where-Object {
    $_ -and
    ($_ -notlike '*w64devkit\bin*') -and
    ($_ -ne "$env:USERPROFILE\.cargo\bin") -and
    ($_ -ne "$env:USERPROFILE\.local\mingw-tools")
}
$env:PATH = (($pathPrefix + $filteredPath) -join ';')

# Provide MinGW gcc/ar for ring's C assembly build without putting w64devkit\bin on PATH (which shadows -lgcc_eh).
$W64DevkitBin = "$env:USERPROFILE\.local\w64devkit\bin"
if ((Test-Path (Join-Path $W64DevkitBin "gcc.exe")) -and -not $env:CC_x86_64_pc_windows_gnu) {
    $env:CC_x86_64_pc_windows_gnu = Join-Path $W64DevkitBin "gcc.exe"
}
if ((Test-Path (Join-Path $W64DevkitBin "ar.exe")) -and -not $env:AR_x86_64_pc_windows_gnu) {
    $env:AR_x86_64_pc_windows_gnu = Join-Path $W64DevkitBin "ar.exe"
}

$PkgJson = Get-Content -LiteralPath (Join-Path $Root "package.json") -Raw -Encoding utf8 | ConvertFrom-Json
$AppVersion = $PkgJson.version

Write-Host "[1/4] Typechecking frontend..." -ForegroundColor Cyan
npm run typecheck
if ($LASTEXITCODE -ne 0) { throw "npm run typecheck failed" }

Write-Host "[2/4] Building MyWinAntify release executable (with embedded dist + icon)..." -ForegroundColor Cyan
npx tauri build --no-bundle
if ($LASTEXITCODE -ne 0) { throw "npx tauri build --no-bundle failed" }

Write-Host "[3/4] Building core-service release executable..." -ForegroundColor Cyan
cargo build --release -p core-service
if ($LASTEXITCODE -ne 0) { throw "cargo build --release -p core-service failed" }

$StageRoot = Join-Path $Root "release"
$AppDir = Join-Path $StageRoot "MyWinAntify"
$ZipPath = Join-Path $StageRoot "MyWinAntify-v$AppVersion-win-x64.zip"

if (Test-Path -LiteralPath $AppDir) {
    Get-ChildItem -LiteralPath $AppDir -Force | Remove-Item -Recurse -Force
} else {
    New-Item -ItemType Directory -Force -Path $AppDir | Out-Null
}
if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }

$DesktopExe = Join-Path $Root "target\release\my-win-antify.exe"
$CoreExe = Join-Path $Root "target\release\core-service.exe"
$WinDivertDll = Join-Path $Root "vendor\windivert\x64\WinDivert.dll"
$WinDivertSys = Join-Path $Root "vendor\windivert\x64\WinDivert64.sys"

$WebView2Loader = Join-Path $Root "target\release\WebView2Loader.dll"
if (-not (Test-Path $WebView2Loader)) {
    $FoundLoader = Get-ChildItem -Path (Join-Path $Root "target\release\build") -Filter "WebView2Loader.dll" -Recurse |
        Where-Object { $_.FullName -like "*\x64\*" } |
        Select-Object -First 1
    if (-not $FoundLoader) {
        throw "Could not find WebView2Loader.dll under target\release"
    }
    $WebView2Loader = $FoundLoader.FullName
}

Copy-Item $DesktopExe (Join-Path $AppDir "MyWinAntify.exe") -Force
Copy-Item $WebView2Loader (Join-Path $AppDir "WebView2Loader.dll") -Force
Copy-Item $CoreExe (Join-Path $AppDir "core-service.exe") -Force
Copy-Item $WinDivertDll (Join-Path $AppDir "WinDivert.dll") -Force
Copy-Item $WinDivertSys (Join-Path $AppDir "WinDivert64.sys") -Force

$Readme = @"
MyWinAntify v$AppVersion (Windows x64 便携绿色版)
=========================================

项目声明：
- 本项目为个人自用与学习研究性质的非官方仿制作品。
- 作者非常喜欢 macOS 端的 Antify（https://antifyapp.com），由于暂无 Windows 版本，
  故尝试在 Windows 上仿照其界面与交互风格实现一个轻量级应用级代理接管工具。
- 当前版本功能尚不完善，仅供学习与技术交流使用，请支持官方正版 Antify！

纯净说明：
- 本发行版完全干净，不包含任何预设代理节点、个人配置或本地运行数据。
- 首次启动后会在 %ProgramData%\MyWinAntify\config.json 自动生成空白默认配置，
  请自行在界面中添加本地上游 SOCKS5 代理（如 127.0.0.1:7890）与应用规则。

一键启动步骤：
1. 将整个 MyWinAntify 文件夹解压到本地磁盘（勿直接在压缩包内双击运行）。
2. 直接双击运行「MyWinAntify.exe」。
3. 首次启动会弹出一次 Windows 管理员权限授权提示（UAC），点击「是」，
   程序会在后台静默拉起流量接管引擎（core-service.exe）与 WinDivert 驱动，无黑框。
4. 在「代理」页添加上游 SOCKS5 代理，在「规则」页添加需要代理的应用，
   回到「仪表盘」点击「启动」即可使用。

退出清理：
- 点击窗口右上角 X 会最小化到系统托盘继续在后台代理。
- 如需彻底关闭并恢复系统直连，请在系统托盘图标右键选择「退出 MyWinAntify」。

运行要求：
- Windows 10 / Windows 11 (x64)
- 需要系统内置 Microsoft Edge WebView2 运行时（Win11 默认自带）。
- 若 Windows Defender 误报拦截 WinDivert64.sys，请将解压后的文件夹加入排除项。
"@
[System.IO.File]::WriteAllText((Join-Path $AppDir "使用说明.txt"), $Readme, [System.Text.UTF8Encoding]::new($true))

Write-Host "[4/4] Creating shareable ZIP archive..." -ForegroundColor Cyan
Compress-Archive -Path $AppDir -DestinationPath $ZipPath -CompressionLevel Optimal -Force

Write-Host "`nPortable package staged at:" -ForegroundColor Green
Get-ChildItem $AppDir | Select-Object Name, Length | Format-Table -AutoSize
$ZipInfo = Get-Item $ZipPath
Write-Host ("ZIP Archive: {0} ({1:N2} MB)" -f $ZipInfo.FullName, ($ZipInfo.Length / 1MB)) -ForegroundColor Green
