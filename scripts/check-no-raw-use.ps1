$hits = rg -n 'format!\("USE \[' src-tauri/src 2>$null
if ($hits) {
    Write-Error "Raw USE [..] interpolation found:`n$hits"
    exit 1
}
Write-Host "check-no-raw-use: OK — no raw USE [..] interpolation"
