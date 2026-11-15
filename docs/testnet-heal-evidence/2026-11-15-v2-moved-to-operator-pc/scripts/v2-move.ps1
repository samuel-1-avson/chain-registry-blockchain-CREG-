# Move validator-2 from VM creg-validator-1 to this PC. Prints no secret values.
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v2move'
$out = Join-Path $dir 'v2-move.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$m) { $log.Add("$((Get-Date).ToUniversalTime().ToString('HH:mm:ss'))Z $m"); $log | Set-Content $out }
function Fail([string]$m) { Add "FAILED: $m"; exit 1 }
function Fp([string]$s) { $h = [Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($s)); (-join ($h | ForEach-Object { $_.ToString('x2') })).Substring(0, 16) }
function Metric([string]$text, [string]$name) {
    $m = [regex]::Match($text, '(?m)^' + [regex]::Escape($name) + '(?:\{[^}]*\})?\s+([-+0-9.eE]+)\s*$')
    if ($m.Success) { return [double]$m.Groups[1].Value } else { return $null }
}

$vm = 'creg-validator-1'
$common = @('--project=gen-lang-client-0022105784', '--zone=us-east1-b', '--tunnel-through-iap', '--strict-host-key-checking=no', '--quiet')
$ref = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$imageId = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$root = 'C:\Users\samue\.creg-phase-c\r7\v2'
$private = 'C:\Users\samue\.creg-phase-c\r7\v2-private'
$data = Join-Path $root 'data'
$scanner = Join-Path $root 'scanner-r14-ansi-regex-live'
$envPath = Join-Path $private 'node.env'
$tar = Join-Path $private 'v2-move.tar'
$name = 'creg-phase-c-r7-v2'
$api = 'http://127.0.0.1:18080'
$tipHash = '730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285'
$genesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$profile = '7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd'
# The run compared a 16-hex SHA-256 fingerprint of CREG_VALIDATOR_KEY here.
# The value is not published; take it from the source node before reusing this script.
$keyFp = '<fingerprint of the source CREG_VALIDATOR_KEY>'
$tarExe = 'C:\Windows\System32\tar.exe'

if (-not (Test-Path $root) -or -not (Test-Path $private)) { Fail 'prep folders missing' }
if (@(Get-ChildItem -Force $root).Count -ne 0 -or @(Get-ChildItem -Force $private).Count -ne 0) { Fail 'prep folders are not empty' }
if (docker ps -a --format '{{.Names}}' | Where-Object { $_ -eq $name }) { Fail "local $name already exists" }
if ((docker image inspect $ref --format '{{.Id}}' | Out-String).Trim() -ne $imageId) { Fail 'image missing locally' }

# 1. Stop and pack on the VM.
foreach ($s in @('v2-stop-pack.sh', 'v2-restore-vm.sh', 'v2-cleanup-vm.sh')) {
    $lf = Join-Path $dir ($s + '.lf')
    [IO.File]::WriteAllText($lf, ((Get-Content -Raw (Join-Path $dir $s)) -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
    $r = gcloud compute scp $lf "${vm}:/tmp/$s" @common 2>&1
    if ($LASTEXITCODE -ne 0) { Fail "copy $s" }
}
$pack = gcloud compute ssh $vm @common '--command=bash /tmp/v2-stop-pack.sh' 2>&1
$packExit = $LASTEXITCODE
foreach ($l in $pack) { Add "vm: $l" }
if ($packExit -ne 0 -or -not (($pack | Out-String) -match 'PACKED:')) { Fail 'stop/pack on the VM failed (the VM script restarts v2 if it had stopped it)' }
$tarSha = [regex]::Match(($pack | Out-String), 'tar_sha256=([0-9a-f]{64})').Groups[1].Value
$idFp = [regex]::Match(($pack | Out-String), 'identity_fp=([0-9a-f]{16})').Groups[1].Value

function Restore-Vm([string]$why) {
    Add "restoring v2 on the VM: $why"
    $x = gcloud compute ssh $vm @common '--command=bash /tmp/v2-restore-vm.sh' 2>&1
    Add "vm restore exit: $LASTEXITCODE $($x -join ' | ')"
    Fail $why
}

# 2. Copy here and check.
$copy = gcloud compute scp "${vm}:/tmp/v2-move.tar" $tar @common 2>&1
if ($LASTEXITCODE -ne 0) { Restore-Vm 'copying the package failed' }
$localSha = (Get-FileHash -LiteralPath $tar -Algorithm SHA256).Hash.ToLowerInvariant()
Add "local tar sha256: $localSha"
if ($localSha -ne $tarSha) { Remove-Item -LiteralPath $tar -Force; Restore-Vm 'package hash mismatch' }

# 3. Unpack: data and scanner to the v2 folder, the key file to the private folder.
$extract = Join-Path $private 'extract'
New-Item -ItemType Directory -Path $extract | Out-Null
& $tarExe -xf $tar -C $extract 2>&1 | ForEach-Object { Add "tar: $_" }
if ($LASTEXITCODE -ne 0) { Remove-Item -Recurse -Force $extract, $tar; Restore-Vm 'unpacking failed' }
Move-Item -LiteralPath (Join-Path $extract 'data') -Destination $data
Move-Item -LiteralPath (Join-Path $extract 'scanner-r14-ansi-regex-live') -Destination $scanner
Move-Item -LiteralPath (Join-Path $extract 'node.env') -Destination $envPath
Remove-Item -Recurse -Force $extract
Remove-Item -Force -LiteralPath $tar
$localIdFp = (Get-FileHash -LiteralPath (Join-Path $data 'p2p\identity.key') -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16)
$envKey = ([IO.File]::ReadAllLines($envPath) | Where-Object { $_ -like 'CREG_VALIDATOR_KEY=*' } | Select-Object -First 1)
$localKeyFp = if ($envKey) { Fp ($envKey.Substring(19).TrimEnd("`r")) } else { 'none' }
Add "identity fp vm=$idFp local=$localIdFp; key fp local=$localKeyFp expected=$keyFp; data files=$(@(Get-ChildItem -Recurse -File $data).Count)"
if ($localIdFp -ne $idFp -or $localKeyFp -ne $keyFp) { Restore-Vm 'identity or key differs after unpacking' }
$r = gcloud compute ssh $vm @common '--command=bash /tmp/v2-cleanup-vm.sh' 2>&1
Add "vm package removed: $($r -join ' ')"

# 4. Start here. From this point the VM copy must stay stopped.
$runArgs = @('run', '-d', '--name', $name, '--restart', 'unless-stopped', '--privileged',
    '--security-opt', 'label=disable', '--env-file', $envPath,
    '-p', '18080:8080', '-p', '9001:9000',
    '--mount', "type=bind,source=$data,target=/data",
    '--mount', "type=bind,source=$scanner,target=/scanner,readonly",
    '--mount', 'type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock',
    $ref)
$cid = docker @runArgs 2>&1
Add "docker run exit: $LASTEXITCODE $cid"
if ($LASTEXITCODE -ne 0) { Fail 'local start failed; v2 is stopped in both places, investigate before restarting either' }
$started = [DateTime]::UtcNow

# 5. Readiness, up to 6 minutes.
$deadline = $started.AddMinutes(6)
$checks = $null
while ([DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Seconds 5
    try {
        $h = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 8
        $s = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
        $p = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
        $m = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8 -UseBasicParsing).Content
        $health = (docker inspect $name --format '{{.State.Health.Status}}' | Out-String).Trim()
        $checks = [ordered]@{
            image = (docker inspect $name --format '{{.Image}}' | Out-String).Trim() -eq $imageId
            docker_healthy = $health -eq 'healthy'
            validator_2 = $h.status -eq 'ok' -and $h.node_id -eq 'validator-2' -and $h.is_validator -eq $true
            chain = $s.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $s.spec_signature_verified -eq $true -and (($s.genesis_hash -replace '^0x', '') -eq $genesis)
            tip = $s.tip_height -ge 20 -and $s.finalized_height -ge 20 -and ($s.tip_height -ne 20 -or $s.tip_hash -eq $tipHash)
            four_validators = $s.validator_count -eq 4 -and $s.active_validators -eq 4
            peers = $s.peer_count -gt 0
            scanner = (Metric $m 'creg_scanner_profile_ready') -eq 1 -and $m.Contains($profile)
            queues = (Metric $m 'creg_pending_block_txs') -eq 0 -and (Metric $m 'creg_durable_finalized_work_depth') -eq 0
            pbft = @($p.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') }).Count -eq 0
            sandbox = $h.sandbox.isolated -eq $true -and $h.sandbox.degraded -eq $false
            identity = (Get-FileHash -LiteralPath (Join-Path $data 'p2p\identity.key') -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16) -eq $idFp
        }
        if (@($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0) { break }
    } catch { $checks = [ordered]@{ error = $_.Exception.Message } }
}
Add "checks: $(($checks | ConvertTo-Json -Compress))"
$logs = (docker logs $name 2>&1 | Out-String) -replace ([string][char]27 + '\[[0-9;]*m'), ''
$peerId = [regex]::Match($logs, 'local_peer_id=([1-9A-HJ-NP-Za-km-z]{40,60})').Groups[1].Value
Add "local peer id: $peerId"
Add "log lines: $(@($logs -split "`n").Count); ERROR lines: $(@($logs -split "`n" | Where-Object { $_ -match ' ERROR ' }).Count)"
$passed = $checks -and -not $checks.Contains('error') -and @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
if (-not $passed) {
    docker update --restart=no $name | Out-Null
    docker stop --time 30 $name | Out-Null
    Fail 'readiness failed; the local v2 is stopped (restart=no) and the VM v2 stays stopped. Investigate before starting either.'
}
$evidence = [ordered]@{
    schema = 'creg-r7-v2-move-to-local-v1'; moved_at_utc = $started.ToString('o')
    from = 'GCP VM creg-validator-1 (gen-lang-client-0022105784, us-east1-b)'; to = 'operator PC (Docker Desktop)'
    node_id = 'validator-2'; validator_pubkey = '779c9522addbce63f78448fa83bff1e789a4a874c4db25e0f7fe00b3d0429fd9'
    image_id = $imageId; node_binary_sha256 = 'c7ef1d3e8bb9b34e2e0e4e255bf47f751ecaada4d0617f9ac5a4dbedb262a5b1'
    package_sha256 = $tarSha; p2p_identity_fp = $idFp; validator_key_fp = $keyFp; local_peer_id = $peerId
    ports = '18080->8080 api, 9001->9000 p2p'; checks = $checks; passed = $true
    vm_container = 'stopped, restart=no'; vm_deleted = $false
}
($evidence | ConvertTo-Json -Depth 6) | Set-Content (Join-Path $dir 'v2-move-evidence.json')
Add 'MOVED: validator-2 runs on this PC'
'finished'
