@echo off
setlocal
cd /d "%~dp0"
where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust is not installed. Get it from https://rustup.rs and run this again.
    pause
    exit /b 1
)
cargo build --release --workspace
if errorlevel 1 (
    echo.
    echo The build failed. See the messages above.
    pause
    exit /b 1
)
if not exist dist mkdir dist
copy /y "target\release\swmm.exe" "dist\swmm.exe" >nul
copy /y "target\release\swmm_loader.dll" "dist\swmm.dll" >nul
powershell -NoProfile -Command "Compress-Archive -Force -Path 'dist\swmm.exe','dist\swmm.dll','LICENSE' -DestinationPath 'dist\swmm-x86_64-windows.zip'"
where wix >nul 2>nul
if errorlevel 1 (
    echo.
    echo Built dist\swmm.exe and dist\swmm-x86_64-windows.zip.
    echo Install WiX to also make the installer: dotnet tool install --global wix --version 5.0.2
    pause
    exit /b 0
)
for /f "tokens=1 delims=+" %%w in ('wix --version') do set WIXVERSION=%%w
wix extension add -g WixToolset.UI.wixext/%WIXVERSION% >nul
for /f "tokens=2 delims== " %%v in ('findstr /b /c:"version = " Cargo.toml') do if not defined VERSION set VERSION=%%~v
wix build assets\installer\swmm.wxs -ext WixToolset.UI.wixext -pdbtype none -arch x64 -d Version=%VERSION% -d Bin=target\release -d Assets=assets -o "dist\swmm-x86_64-windows.msi"
if errorlevel 1 (
    echo.
    echo Making the installer failed. See the messages above.
    pause
    exit /b 1
)
echo.
echo Done. Files in %~dp0dist:
echo   swmm.exe and swmm.dll
echo   swmm-x86_64-windows.zip
echo   swmm-x86_64-windows.msi
pause
