# Steps 1-2 of the validator-1 restore. Read-only; prints public keys only.
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v1restore'
$out = Join-Path $dir 'v1-verify-key.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$label, $lines) { $log.Add("== $label"); foreach ($l in @($lines)) { $log.Add([string]$l) }; $log | Set-Content $out }
$want = 'cc4157c4443c9fb011df2b650f497e3eadfc1ddda6dc7f4b6732325383108ffc'

Add 'node version' (node --version 2>&1)
$secret = (gcloud secrets versions access 1 --secret=creg-fresh-testnet-validator-1-ed25519 --project=gen-lang-client-0022105784 --quiet 2>$null | Out-String)
$accessExit = $LASTEXITCODE
Add 'secret access exit' $accessExit
if ($accessExit -eq 0 -and $secret) {
    $res = $secret | node (Join-Path $dir 'derive-ed25519-pub.js') 2>&1
    Add 'derived from secret' $res
    $j = ($res | Out-String) | ConvertFrom-Json
    Add 'matches R7 validator-1' ($j.ok -and $j.pubkey -eq $want)
}
$secret = $null
[GC]::Collect()

$lf = Join-Path $dir 'fleet-keys.lf.sh'
[IO.File]::WriteAllText($lf, ((Get-Content -Raw (Join-Path $dir 'fleet-keys.sh')) -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
$hosts = @(
    @{ Vm = 'creg-validator-3'; P = 'fiffi-500411'; Z = 'europe-west1-b' },
    @{ Vm = 'creg-validator-4'; P = 'fiffi-500411'; Z = 'asia-southeast1-b' },
    @{ Vm = 'creg-testnet-vm'; P = 'gen-lang-client-0022105784'; Z = 'us-central1-b' },
    @{ Vm = 'creg-observer-vm'; P = 'gen-lang-client-0022105784'; Z = 'us-central1-a' },
    @{ Vm = 'creg-observer-3'; P = 'fiffi-500411'; Z = 'us-west1-b' },
    @{ Vm = 'creg-observer-4'; P = 'fiffi-500411'; Z = 'africa-south1-a' }
)
foreach ($h in $hosts) {
    $common = @("--project=$($h.P)", "--zone=$($h.Z)", '--tunnel-through-iap', '--strict-host-key-checking=no', '--quiet')
    $r = gcloud compute scp $lf "$($h.Vm):/tmp/fleet-keys.sh" @common 2>&1
    if ($LASTEXITCODE -ne 0) { Add "$($h.Vm) scp failed" ($r | Select-Object -Last 2); continue }
    Add $h.Vm (gcloud compute ssh $h.Vm @common '--command=bash /tmp/fleet-keys.sh; rm -f /tmp/fleet-keys.sh' 2>&1)
}
Add 'local containers with a validator key' (docker ps -a --format '{{.Names}} {{.Status}}' | Where-Object { $_ -match 'creg' })
'finished'
