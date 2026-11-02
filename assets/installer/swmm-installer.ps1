$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$repo = "__REPO__"
$tag = "__TAG__"
$asset = "swmm-x86_64-windows.zip"
$url = "https://github.com/$repo/releases/download/$tag/$asset"
$installDir = Join-Path $env:LOCALAPPDATA "Programs\SWMM"
$temp = Join-Path ([IO.Path]::GetTempPath()) ("swmm-" + [guid]::NewGuid())

Write-Host "Downloading SWMM $tag..."
New-Item -ItemType Directory -Force -Path $temp | Out-Null
$zip = Join-Path $temp $asset
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

Get-Process -Name "swmm" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Expand-Archive -Path $zip -DestinationPath $installDir -Force
Remove-Item -Recurse -Force $temp

$programs = [Environment]::GetFolderPath("Programs")
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $programs "SWMM.lnk"))
$shortcut.TargetPath = Join-Path $installDir "swmm.exe"
$shortcut.WorkingDirectory = $installDir
$shortcut.Description = "Savage Wind Mod Manager"
$shortcut.Save()

Write-Host "SWMM $tag is installed in $installDir"
Write-Host "Open it from the Start menu."
