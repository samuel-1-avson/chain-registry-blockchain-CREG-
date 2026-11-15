# Snapshot, then delete VM creg-validator-1. Confirmed by the user 2026-11-15.
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v2move'
$out = Join-Path $dir 'v2-delete-vm.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$m) { $log.Add("$((Get-Date).ToUniversalTime().ToString('HH:mm:ss'))Z $m"); $log | Set-Content $out }
function Fail([string]$m) { Add "FAILED: $m"; exit 1 }
$p = '--project=gen-lang-client-0022105784'; $z = '--zone=us-east1-b'
$vm = 'creg-validator-1'; $disk = 'creg-validator-1'; $snap = 'creg-validator-1-v2-moved-20261115'

# Guard: the PC copy must be healthy before the cloud one goes.
try {
    $h = Invoke-RestMethod -Uri 'http://127.0.0.1:18080/v1/public/health' -TimeoutSec 8
    $s = Invoke-RestMethod -Uri 'http://127.0.0.1:18080/v1/public/chain/stats' -TimeoutSec 8
} catch { Fail "PC v2 not reachable: $($_.Exception.Message)" }
Add "PC v2: status=$($h.status) node=$($h.node_id) validator=$($h.is_validator) tip=$($s.tip_height) peers=$($s.peer_count)"
if ($h.status -ne 'ok' -or $h.node_id -ne 'validator-2' -or -not $h.is_validator -or $s.peer_count -lt 1) { Fail 'PC v2 is not healthy; not deleting' }

$desc = (gcloud compute instances describe $vm $p $z '--format=value(status,disks[0].source.basename(),disks[0].autoDelete)' --quiet 2>&1 | Out-String).Trim()
Add "vm before: $desc"
if ($desc -notmatch "^(RUNNING|TERMINATED)\s+$disk\s+True$") { Fail "unexpected VM state: $desc" }

$r = gcloud compute instances stop $vm $p $z --quiet 2>&1
Add "stop exit: $LASTEXITCODE"
if ($LASTEXITCODE -ne 0) { Fail ($r -join ' | ') }

$r = gcloud compute disks snapshot $disk $p $z "--snapshot-names=$snap" '--description=Backup of creg-validator-1 after validator-2 moved to the operator PC on 2026-11-15, holds the validator-2 key and pre-move data' --quiet 2>&1
Add "snapshot exit: $LASTEXITCODE"
if ($LASTEXITCODE -ne 0) { Fail ($r -join ' | ') }
$status = ''
for ($i = 0; $i -lt 60; $i++) {
    $status = (gcloud compute snapshots describe $snap $p '--format=value(status)' --quiet 2>&1 | Out-String).Trim()
    if ($status -eq 'READY') { break }
    Start-Sleep -Seconds 10
}
$info = (gcloud compute snapshots describe $snap $p '--format=value(status,diskSizeGb,storageBytes,storageLocations.list(),sourceDisk.basename())' --quiet 2>&1 | Out-String).Trim()
Add "snapshot: $info"
if ($status -ne 'READY') { Fail 'snapshot not READY; VM left stopped, not deleted' }

$r = gcloud compute instances delete $vm $p $z --quiet 2>&1
Add "delete exit: $LASTEXITCODE $($r -join ' | ')"
if ($LASTEXITCODE -ne 0) { Fail 'delete failed' }

Add "vm after: [$((gcloud compute instances list $p "--filter=name=$vm" '--format=value(name,status)' --quiet 2>&1 | Out-String).Trim())]"
Add "disk after: [$((gcloud compute disks list $p "--filter=name=$disk" '--format=value(name)' --quiet 2>&1 | Out-String).Trim())]"
Add "snapshot after: [$((gcloud compute snapshots list $p "--filter=name=$snap" '--format=value(name,status)' --quiet 2>&1 | Out-String).Trim())]"
Add "ip after: [$((gcloud compute addresses list $p '--filter=address=34.23.56.23' '--format=value(name)' --quiet 2>&1 | Out-String).Trim())]"
Add 'DONE'
'finished'
