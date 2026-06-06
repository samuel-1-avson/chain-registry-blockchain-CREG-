# Kubo HTTP API helpers (Windows-friendly — avoids Invoke-WebRequest 403 on /api/v0/*).

function Test-CregIpfsApi {
    param([string]$BaseUrl = "http://127.0.0.1:5001")
    $url = "$($BaseUrl.TrimEnd('/'))/api/v0/version"
    if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
        return $false
    }
    $null = & curl.exe -sf -X POST $url 2>$null
    return ($LASTEXITCODE -eq 0)
}
