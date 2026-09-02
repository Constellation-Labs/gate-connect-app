# Settles the open questions in docs/schannel-revocation-analysis.md section 7.
# Run in an elevated-ish PowerShell on the Windows box WITH Gate's proxy ON and
# the CA trusted. Read-only except where noted; changes nothing persistent.

$ErrorActionPreference = 'Continue'
$Host_ = 'api.anthropic.com'   # or claude.ai if claude-web is enabled

Write-Host "`n=== Q0: is the host actually intercepted? ===" -ForegroundColor Cyan
# Issuer should be 'Gate Connect Local CA' if MITM is active on this host.
& certutil -verify -urlfetch (New-TemporaryFile) 2>&1 | Out-Null
$leaf = "$env:TEMP\gate-leaf.cer"
& openssl s_client -connect "${Host_}:443" -servername $Host_ -showcerts 2>$null |
    Out-File "$env:TEMP\gate-chain.pem"
Select-String -Path "$env:TEMP\gate-chain.pem" -Pattern 'issuer=' | Select-Object -First 2

Write-Host "`n=== Q1+Q2: full chain + revocation, with URL fetching ===" -ForegroundColor Cyan
# Look for: which cert in the chain reports the error, and whether a CRL URL
# was attempted. 0x80092012 on the LEAF only => root is not checked (good).
# 0x80092012 on the ROOT too  => root needs a CDP as well (expensive path).
& certutil -verify -urlfetch $leaf

Write-Host "`n=== Q3: what is in the CryptoAPI URL cache? ===" -ForegroundColor Cyan
& certutil -urlcache CRL
Write-Host "--- (to clear while testing: certutil -urlcache CRL delete) ---"

Write-Host "`n=== Q2b: does CryptoAPI fetch a loopback CDP at all? ===" -ForegroundColor Cyan
# Serve a CRL on loopback, then ask CryptoAPI to fetch that exact URL.
# Replace <PACPORT> with the engine's PAC port (see the persisted
# proxy\pac-port file under %LOCALAPPDATA%\Gate Connect).
$pacPort = Get-Content "$env:LOCALAPPDATA\Gate Connect\proxy\pac-port" -EA SilentlyContinue
Write-Host "PAC port: $pacPort"
if ($pacPort) { & certutil -verifyctl -urlfetch "http://127.0.0.1:$pacPort/gate-ca.crl" }

Write-Host "`n=== Q4: which clients hard-fail? ===" -ForegroundColor Cyan
$url = "https://$Host_/v1/models"

Write-Host "`n-- System32 curl (schannel), default --"
& "$env:SystemRoot\System32\curl.exe" -sS -o NUL -w "exit=%{http_code}`n" $url
Write-Host "-- System32 curl, --ssl-revoke-best-effort --"
& "$env:SystemRoot\System32\curl.exe" -sS --ssl-revoke-best-effort -o NUL -w "exit=%{http_code}`n" $url

Write-Host "`n-- PowerShell $($PSVersionTable.PSVersion) Invoke-WebRequest --"
try { (Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 15).StatusCode }
catch { "FAILED: $($_.Exception.Message)" }

Write-Host "`n-- .NET HttpClient, revocation explicitly ON --"
Add-Type -AssemblyName System.Net.Http
$h = [System.Net.Http.HttpClientHandler]::new()
$h.CheckCertificateRevocationList = $true
try { ([System.Net.Http.HttpClient]::new($h)).GetAsync($url).Result.StatusCode }
catch { "FAILED: $($_.Exception.InnerException.Message)" }

Write-Host "`n-- .NET HttpClient, default (revocation OFF) --"
$h2 = [System.Net.Http.HttpClientHandler]::new()
try { ([System.Net.Http.HttpClient]::new($h2)).GetAsync($url).Result.StatusCode }
catch { "FAILED: $($_.Exception.InnerException.Message)" }

Write-Host "`n-- WinHTTP (no revocation flag set) --"
$w = New-Object -ComObject WinHttp.WinHttpRequest.5.1
try { $w.Open('GET', $url, $false); $w.Send(); $w.Status }
catch { "FAILED: $($_.Exception.Message)" }

Write-Host "`nDone. Record which rows fail and with which HRESULT." -ForegroundColor Green
