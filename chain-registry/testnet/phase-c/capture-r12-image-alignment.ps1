param(
    [Parameter(Mandatory = $true)] [string]$EvidencePath
)

$ErrorActionPreference = 'Stop'
$expectedImages = @(
    'sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd',
    'sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736'
)
$expectedTag = 'creg-node:phase-c-secure-r12-bounded-l1-retry'
$expectedTip = '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$nodes = @(
    @{ Slot='v1'; Role='validator'; Name='operator laptop'; Port=18080 },
    @{ Slot='v2'; Role='validator'; Name='creg-validator-1'; Project='gen-lang-client-0022105784'; Zone='us-east1-b'; Port=18080 },
    @{ Slot='v3'; Role='validator'; Name='creg-validator-3'; Project='fiffi-500411'; Zone='europe-west1-b'; Port=8080 },
    @{ Slot='v4'; Role='validator'; Name='creg-validator-4'; Project='fiffi-500411'; Zone='asia-southeast1-b'; Port=8080 },
    @{ Slot='o1'; Role='observer'; Name='creg-testnet-vm'; Project='gen-lang-client-0022105784'; Zone='us-central1-b'; Port=18080 },
    @{ Slot='o2'; Role='observer'; Name='creg-observer-vm'; Project='gen-lang-client-0022105784'; Zone='us-central1-a'; Port=8080 },
    @{ Slot='o3'; Role='observer'; Name='creg-observer-3'; Project='fiffi-500411'; Zone='us-west1-b'; Port=8080 },
    @{ Slot='o4'; Role='observer'; Name='creg-observer-4'; Project='fiffi-500411'; Zone='africa-south1-a'; Port=8080 }
)
$remoteTemplate = @'
python3 - <<'PY'
import hashlib, json, pathlib, subprocess
slot = 'SLOT'
role = 'ROLE'
container = 'creg-phase-c-r7-' + slot
raw = subprocess.check_output(['sudo', 'docker', 'inspect', container], text=True)
info = json.loads(raw)[0]
tag = info['Config']['Image']
image_raw = subprocess.check_output(['sudo', 'docker', 'image', 'inspect', tag], text=True)
image_id = json.loads(image_raw)[0]['Id']
mounts = [{'source': m['Source'], 'destination': m['Destination'], 'rw': m['RW']} for m in info['Mounts']]
data = next((m for m in mounts if m['destination'] == '/data'), None)
scanner = next((m for m in mounts if m['destination'] == '/scanner'), None)
identity = pathlib.Path(data['source']) / 'p2p' / 'identity.key' if data else None
journal = pathlib.Path(data['source']) / 'pbft-safety.rocksdb' if data and role == 'validator' else None
names = subprocess.check_output(['sudo', 'docker', 'ps', '-a', '--format', '{{.Names}}'], text=True).splitlines()
previous = [name for name in names if name.startswith(container + '-') and name.endswith('-preserved')]
identity_hash = subprocess.check_output(['sudo', 'sha256sum', str(identity)], text=True).split()[0] if identity else None
journal_present = subprocess.run(['sudo', 'test', '-d', str(journal)]).returncode == 0 if journal else None
record = {
    'slot': slot, 'role': role, 'state': info['State']['Status'],
    'container_image_id': info['Image'], 'tag_image_id': image_id,
    'configured_image': tag, 'mounts': mounts,
    'p2p_identity_sha256': identity_hash,
    'signing_journal_present': journal_present if role == 'validator' else None,
    'preserved_prior_containers': previous,
}
print(json.dumps(record))
PY
'@

$records = @()
foreach ($node in $nodes) {
    if ($node.Slot -eq 'v1') {
        $raw = (& docker inspect "creg-phase-c-r7-$($node.Slot)" | Out-String)
        if ($LASTEXITCODE -ne 0) { throw 'Could not inspect local V1 container.' }
        $info = @($raw | ConvertFrom-Json)[0]
        $tag = $info.Config.Image
        $tagId = (& docker image inspect $tag --format '{{.Id}}' | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the local R12 image tag.' }
        $mounts = @($info.Mounts | ForEach-Object { [ordered]@{ source=$_.Source; destination=$_.Destination; rw=$_.RW } })
        $data = @($mounts | Where-Object { $_.destination -eq '/data' })[0]
        $identity = Join-Path $data.source 'p2p\identity.key'
        $journal = Join-Path $data.source 'pbft-safety.rocksdb'
        $preserved = @((docker ps -a --format '{{.Names}}') | Where-Object { $_ -like "creg-phase-c-r7-v1-*-preserved" })
        $record = [ordered]@{
            slot='v1'; role='validator'; state=$info.State.Status
            container_image_id=$info.Image; tag_image_id=$tagId; configured_image=$tag
            mounts=$mounts
            p2p_identity_sha256=if (Test-Path -LiteralPath $identity) { (Get-FileHash -LiteralPath $identity -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
            signing_journal_present=Test-Path -LiteralPath $journal
            preserved_prior_containers=$preserved
        }
    } else {
        $command = $remoteTemplate.Replace('SLOT', $node.Slot).Replace('ROLE', $node.Role)
        $output = (& gcloud compute ssh $node.Name "--project=$($node.Project)" "--zone=$($node.Zone)" --tunnel-through-iap --quiet "--command=$command" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Live image inspection failed for $($node.Slot)." }
        try { $record = $output | ConvertFrom-Json -AsHashtable }
        catch { throw "Image inspection output for $($node.Slot) was not valid JSON." }
    }
    $statsUrl = "http://127.0.0.1:$($node.Port)/v1/public/chain/stats"
    if ($node.Slot -eq 'v1') {
        $stats = Invoke-RestMethod -Uri $statsUrl -TimeoutSec 10
    } else {
        $remoteStats = "sudo docker exec creg-phase-c-r7-$($node.Slot) curl -fsS --max-time 8 http://127.0.0.1:8080/v1/public/chain/stats"
        $statsText = (& gcloud compute ssh $node.Name "--project=$($node.Project)" "--zone=$($node.Zone)" --tunnel-through-iap --quiet "--command=$remoteStats" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Live chain inspection failed for $($node.Slot)." }
        $stats = $statsText | ConvertFrom-Json -AsHashtable
    }
    $mounts = @($record.mounts)
    $dataMount = @($mounts | Where-Object { $_.destination -eq '/data' })[0]
    $scannerMount = @($mounts | Where-Object { $_.destination -eq '/scanner' })[0]
    $checks = [ordered]@{
        container_running = $record.state -eq 'running'
        same_r12_image_family = $record.configured_image -eq $expectedTag -and $record.container_image_id -in $expectedImages -and $record.tag_image_id -in $expectedImages
        persistent_data_mounted = $null -ne $dataMount
        scanner_mounted_readonly = $null -ne $scannerMount -and $scannerMount.rw -eq $false
        p2p_identity_present = -not [string]::IsNullOrWhiteSpace([string]$record.p2p_identity_sha256)
        preserved_previous_container = @($record.preserved_prior_containers).Count -gt 0
        signer_journal_present = $node.Role -ne 'validator' -or $record.signing_journal_present -eq $true
        chain_tip_retained = $stats.tip_height -eq 4 -and $stats.tip_hash -eq $expectedTip
    }
    $record['chain'] = [ordered]@{ chain_id=$stats.chain_id; genesis_hash=$stats.genesis_hash; tip_height=$stats.tip_height; tip_hash=$stats.tip_hash; peer_count=$stats.peer_count }
    $record['checks'] = $checks
    $record['passed'] = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    $records += $record
    "$($node.Slot): image/alignment passed=$($record.passed)"
}
$evidence = [ordered]@{
    schema='creg-real-r12-fleet-image-alignment-v1'
    captured_at_utc=[DateTime]::UtcNow.ToString('o')
    image_tag=$expectedTag
    accepted_image_ids=$expectedImages
    all_eight_aligned=$records.Count -eq 8 -and @($records | Where-Object { -not $_.passed }).Count -eq 0
    public_cutover=$false
    nodes=$records
}
$evidence | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $evidence.all_eight_aligned) { throw "R12 alignment checks failed. Evidence: $EvidencePath" }
"All eight real nodes are aligned to r12: $EvidencePath"
