# Restore validator-1 on this PC with its original key (plan Option A).
# Prints no secret values.
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v1restore'
$out = Join-Path $dir 'v1-restore.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$m) { $log.Add("$((Get-Date).ToUniversalTime().ToString('HH:mm:ss'))Z $m"); $log | Set-Content $out }
function Fail([string]$m) { Add "FAILED: $m"; exit 1 }
function Metric([string]$text, [string]$name) {
    $m = [regex]::Match($text, '(?m)^' + [regex]::Escape($name) + '(?:\{[^}]*\})?\s+([-+0-9.eE]+)\s*$')
    if ($m.Success) { return [double]$m.Groups[1].Value } else { return $null }
}
$ref = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$imageId = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$base = 'C:\Users\samue\.creg-phase-c\r7'
$data = Join-Path $base 'v1\data'
$private = Join-Path $base 'v1-private'
$envPath = Join-Path $private 'node.env'
$v2Env = Join-Path $base 'v2-private\node.env'
$scanner = Join-Path $base 'v2\scanner-r14-ansi-regex-live'
$name = 'creg-phase-c-r7-v1'
$api = 'http://127.0.0.1:18081'
$want = 'cc4157c4443c9fb011df2b650f497e3eadfc1ddda6dc7f4b6732325383108ffc'
$genesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$tipHash = '730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285'
$profile = '7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd'
$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssZ')
$backup = Join-Path $base "v1-data-before-restore-$stamp"

# Preconditions
foreach ($p in @($data, (Join-Path $data 'pbft-safety.rocksdb'), (Join-Path $data 'p2p\identity.key'), $v2Env, $scanner)) { if (-not (Test-Path -LiteralPath $p)) { Fail "missing $p" } }
if (Test-Path -LiteralPath $private) { Fail "$private already exists" }
if (docker ps -a --format '{{.Names}}' | Where-Object { $_ -eq $name }) { Fail "a $name container already exists" }
if (@(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalPort -in 18081, 9002 }).Count -ne 0) { Fail 'port 18081 or 9002 is in use' }
if ((docker image inspect $ref --format '{{.Id}}' | Out-String).Trim() -ne $imageId) { Fail 'image missing' }
$idFp = (Get-FileHash -LiteralPath (Join-Path $data 'p2p\identity.key') -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16)
Add "v1 identity fp: $idFp"

# 3a. Back up the data folder and check the copy.
Copy-Item -LiteralPath $data -Destination $backup -Recurse
$src = Get-ChildItem -LiteralPath $data -Recurse -File | Measure-Object Length -Sum
$dst = Get-ChildItem -LiteralPath $backup -Recurse -File | Measure-Object Length -Sum
Add "backup: $backup files=$($dst.Count)/$($src.Count) bytes=$($dst.Sum)/$($src.Sum)"
if ($src.Count -ne $dst.Count -or $src.Sum -ne $dst.Sum) { Fail 'backup copy differs' }

# 3b. Fetch the key, check it, and build node.env in a private folder.
$secret = (gcloud secrets versions access 1 --secret=creg-fresh-testnet-validator-1-ed25519 --project=gen-lang-client-0022105784 --quiet 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or -not $secret) { Fail 'could not read the validator-1 key' }
$check = ($secret | node (Join-Path $dir 'derive-ed25519-pub.js') | Out-String) | ConvertFrom-Json
if (-not $check.ok -or $check.pubkey -ne $want -or $check.format -ne 'plain-hex' -or $check.bytes -ne 32) { $secret = $null; Fail 'secret is not validator-1''s 32-byte key' }
Add "key verified: pubkey $($check.pubkey)"
$lines = [IO.File]::ReadAllLines($v2Env) | ForEach-Object { $_.TrimEnd("`r") }
if (@($lines | Where-Object { $_ -eq 'CREG_NODE_ID=validator-2' }).Count -ne 1) { $secret = $null; Fail 'v2 env has no single CREG_NODE_ID=validator-2 line' }
if (@($lines | Where-Object { $_ -like 'CREG_VALIDATOR_KEY=*' }).Count -ne 1) { $secret = $null; Fail 'v2 env has no single CREG_VALIDATOR_KEY line' }
$v1Lines = foreach ($l in $lines) {
    if ($l -eq 'CREG_NODE_ID=validator-2') { 'CREG_NODE_ID=validator-1' }
    elseif ($l -like 'CREG_VALIDATOR_KEY=*') { 'CREG_VALIDATOR_KEY=' + $secret }
    else { $l }
}
$secret = $null
New-Item -ItemType Directory -Path $private | Out-Null
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
icacls.exe $private /inheritance:r /grant:r "*$($sid):(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' | Out-Null
if ($LASTEXITCODE -ne 0) { Fail 'could not restrict the private folder' }
[IO.File]::WriteAllText($envPath, (($v1Lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
$v1Lines = $null
[GC]::Collect()
$envCheck = [IO.File]::ReadAllLines($envPath)
Add "env written: $($envCheck.Count) lines; node id line: $(@($envCheck | Where-Object { $_ -eq 'CREG_NODE_ID=validator-1' }).Count); differs from v2 only in node id and key: $(@(Compare-Object $lines $envCheck).Count -eq 4)"
$envCheck = $null

# 4. Start v1.
$runArgs = @('run', '-d', '--name', $name, '--restart', 'unless-stopped', '--privileged',
    '--security-opt', 'label=disable', '--env-file', $envPath,
    '-p', '18081:8080', '-p', '9002:9000',
    '--mount', "type=bind,source=$data,target=/data",
    '--mount', "type=bind,source=$scanner,target=/scanner,readonly",
    '--mount', 'type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock',
    $ref)
$cid = docker @runArgs 2>&1
Add "docker run exit: $LASTEXITCODE $cid"
if ($LASTEXITCODE -ne 0) { Fail 'start failed' }
$started = [DateTime]::UtcNow

$deadline = $started.AddMinutes(8)
$checks = $null
while ([DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Seconds 5
    $state = (docker inspect $name --format '{{.State.Status}} {{.RestartCount}}' | Out-String).Trim()
    if ($state -notmatch '^running 0$') { $checks = [ordered]@{ error = "container state: $state" }; if ($state -match 'restarting|exited') { break } else { continue } }
    try {
        $h = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 8
        $s = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
        $p = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
        $m = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8 -UseBasicParsing).Content
        $checks = [ordered]@{
            image = (docker inspect $name --format '{{.Image}}' | Out-String).Trim() -eq $imageId
            docker_healthy = (docker inspect $name --format '{{.State.Health.Status}}' | Out-String).Trim() -eq 'healthy'
            validator_1 = $h.status -eq 'ok' -and $h.node_id -eq 'validator-1' -and $h.is_validator -eq $true
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
$journalErr = @($logs -split "`n" | Where-Object { $_ -match 'PBFT safety database|different genesis or validator identity|identity anchor' })
Add "journal identity errors: $($journalErr.Count) $($journalErr -join ' | ')"
$peerId = [regex]::Match($logs, 'local_peer_id=([1-9A-HJ-NP-Za-km-z]{40,60})').Groups[1].Value
Add "local peer id: $peerId"
Add "log lines: $(@($logs -split "`n").Count); ERROR lines: $(@($logs -split "`n" | Where-Object { $_ -match ' ERROR ' }).Count)"
Add "last lines: $((@($logs -split "`n" | Where-Object { $_.Trim() }) | Select-Object -Last 3) -join ' || ')"
$passed = $checks -and -not $checks.Contains('error') -and @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0 -and $journalErr.Count -eq 0
if (-not $passed) {
    docker update --restart=no $name | Out-Null
    docker stop --time 30 $name | Out-Null
    Fail 'readiness failed; v1 is stopped (restart=no). Nothing else was changed.'
}
Add "RESTORED: validator-1 runs on this PC; backup $backup"
'finished'
