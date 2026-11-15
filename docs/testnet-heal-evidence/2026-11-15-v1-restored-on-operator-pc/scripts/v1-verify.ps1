$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v1restore'
$out = Join-Path $dir 'v1-verify.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$label, $lines) { $log.Add("== $label"); foreach ($l in @($lines)) { $log.Add([string]$l) }; $log | Set-Content $out }
$v1 = '12D3KooWMpALWEjVhPTJ8Hjxk3ZVYeYXhX8LkvwrjCjjzTSmFvXA'
$v2 = '12D3KooWChgcSfZ7erY2BaXPy8e4PPLvtBEBWZ2eD21P9RQxkxCk'
$lf = Join-Path $dir 'peer-seen.lf.sh'
[IO.File]::WriteAllText($lf, ((Get-Content -Raw (Join-Path $dir 'peer-seen.sh')) -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
$nodes = @(
    @{ Vm = 'creg-validator-3'; Z = 'europe-west1-b'; C = 'creg-phase-c-r7-v3' },
    @{ Vm = 'creg-validator-4'; Z = 'asia-southeast1-b'; C = 'creg-phase-c-r7-v4' },
    @{ Vm = 'creg-observer-4'; Z = 'africa-south1-a'; C = 'creg-phase-c-r7-o4' }
)
foreach ($n in $nodes) {
    $common = @('--project=fiffi-500411', "--zone=$($n.Z)", '--tunnel-through-iap', '--strict-host-key-checking=no', '--quiet')
    $r = gcloud compute scp $lf "$($n.Vm):/tmp/peer-seen.sh" @common 2>&1
    Add $n.Vm (gcloud compute ssh $n.Vm @common "--command=bash /tmp/peer-seen.sh $($n.C) 8080 2026-11-15T11:27:00Z $v1 $v2" 2>&1)
}
foreach ($pair in @(@('v1', 18081), @('v2', 18080))) {
    try {
        $h = Invoke-RestMethod -Uri "http://127.0.0.1:$($pair[1])/v1/public/health" -TimeoutSec 8
        $s = Invoke-RestMethod -Uri "http://127.0.0.1:$($pair[1])/v1/public/chain/stats" -TimeoutSec 8
        Add "PC $($pair[0])" "status=$($h.status) node=$($h.node_id) validator=$($h.is_validator) tip=$($s.tip_height) hash=$($s.tip_hash.Substring(0,8)) peers=$($s.peer_count)"
    } catch { Add "PC $($pair[0])" "error $($_.Exception.Message)" }
}
Add 'PC containers' (docker ps --filter name=creg-phase-c-r7 --format '{{.Names}} {{.Status}} {{.Ports}}')
Add 'PC docker memory' (docker stats --no-stream --format '{{.Name}} {{.MemUsage}} {{.CPUPerc}}' creg-phase-c-r7-v1 creg-phase-c-r7-v2 2>&1)
'finished'
