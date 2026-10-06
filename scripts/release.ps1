# Сборка тестовых экземпляров moth-amp:
#   Windows — папка dist\moth-amp-<версия>-windows-x64\ и zip рядом (без установки);
#   Android — release-APK по архитектурам (для телефонов нужен arm64-v8a).
#
# Запуск из корня репозитория:
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -SkipAndroid
#
# APK подписан отладочным ключом Flutter — для тестов. Для публикации нужен
# свой ключ (android\key.properties).

param(
    [switch]$SkipWindows,
    [switch]$SkipAndroid
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$app = Join-Path $root 'app'
$dist = Join-Path $root 'dist'

$versionLine = (Select-String -Path (Join-Path $app 'pubspec.yaml') -Pattern '^version:\s*(.+)$').Matches[0]
$version = $versionLine.Groups[1].Value.Trim().Split('+')[0]

if (-not (Get-Command flutter -ErrorAction SilentlyContinue)) { $env:Path += ';C:\src\flutter\bin' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:Path += ";$env:USERPROFILE\.cargo\bin" }

New-Item -ItemType Directory -Force $dist | Out-Null
Push-Location $app
try {
    if (-not $SkipWindows) {
        Write-Host "== Windows $version"
        flutter build windows --release
        if ($LASTEXITCODE -ne 0) { throw 'Сборка Windows не удалась' }
        $out = Join-Path $dist "moth-amp-$version-windows-x64"
        if (Test-Path $out) { Remove-Item -Recurse -Force $out }
        Copy-Item -Recurse (Join-Path $app 'build\windows\x64\runner\Release') $out
        Compress-Archive -Path (Join-Path $out '*') -DestinationPath "$out.zip" -Force
    }

    if (-not $SkipAndroid) {
        Write-Host "== Android $version"
        if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = 'C:\Android\sdk' }
        if (-not $env:JAVA_HOME) { $env:JAVA_HOME = 'C:\Android\jdk17' }
        flutter build apk --release --split-per-abi
        if ($LASTEXITCODE -ne 0) { throw 'Сборка Android не удалась' }
        Get-ChildItem (Join-Path $app 'build\app\outputs\flutter-apk') -Filter 'app-*-release.apk' | ForEach-Object {
            $abi = $_.BaseName -replace '^app-', '' -replace '-release$', ''
            Copy-Item $_.FullName (Join-Path $dist "moth-amp-$version-android-$abi.apk") -Force
        }
    }
}
finally {
    Pop-Location
}

Write-Host "== Готово: $dist"
Get-ChildItem $dist | Select-Object Name, @{ n = 'МБ'; e = { [math]::Round($_.Length / 1MB, 1) } } | Format-Table -AutoSize
