param(
 [string]$PrivateDirectory='C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\r18-event-publish',
 [string]$EvidenceDirectory=(Join-Path $PSScriptRoot '../../../docs/testnet-heal-evidence/2026-09-26-phase-c-events-r16')
)
$ErrorActionPreference='Stop'
$image='creg-node:phase-c-secure-r18-canonical-round-recovery'
$pin=(& docker image inspect $image --format '{{.Id}}'|Out-String).Trim()
if($pin -notin @('sha256:cee77b5210ad67a91f5d6e5ccbf2b5ec914ccf53268f8294b261bc7a37c7f49a','sha256:dd07eef4a657707fdad696b1b4cc7397ee2589b82e3511c892ff3a806f3dcb03')){throw 'Reviewed R18 CLI image required.'}
$source=Join-Path $EvidenceDirectory 'next-package/toidentifier-1.0.1.tgz'
if((Get-FileHash $source).Hash.ToLowerInvariant() -ne '186fcc77488de327daf911d362d4e773bab9909f1df2a5f0c20b875205b92e08'){throw 'Reviewed real npm artifact changed.'}
$specDirectory=(Resolve-Path (Join-Path $PSScriptRoot 'r7')).Path
$spec=Get-Content (Join-Path $specDirectory 'chain-spec.json') -Raw|ConvertFrom-Json
if($spec.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7'){throw 'Wrong chain context.'}
$requestPath=Join-Path $PrivateDirectory 'signed-publish-toidentifier-r7.json'
if(Test-Path $requestPath){throw 'Existing signed authorization must be preserved; do not renew blindly.'}
$keyFile='C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\publisher-ed25519.key'
if(-not(Test-Path $keyFile)){throw 'Protected publisher key missing.'}
New-Item -ItemType Directory -Path $PrivateDirectory -Force|Out-Null
Copy-Item $source (Join-Path $PrivateDirectory 'toidentifier-1.0.1.tgz')
Copy-Item (Join-Path $EvidenceDirectory 'next-package/toidentifier-manifest.json') (Join-Path $PrivateDirectory 'creg-manifest.json')
$savedKey=[Environment]::GetEnvironmentVariable('CREG_PUBLISHER_EVM_KEY','Process')
try{
 $secret=(& gcloud secrets versions access latest --secret=creg-fresh-testnet-publisher-evm --project=gen-lang-client-0022105784 2>$null|Out-String).Trim()
 if($LASTEXITCODE -ne 0 -or $secret -notmatch '^0x[0-9a-fA-F]{64}$'){throw 'Could not securely load fresh publisher key.'}
 $env:CREG_PUBLISHER_EVM_KEY=$secret; $secret=$null
 & docker run --rm --entrypoint /app/creg --env CREG_PUBLISHER_EVM_KEY --env XDG_CACHE_HOME=/tmp/creg-cache `
   --env "CREG_CHAIN_ID=$($spec.chain_id)" --env "CREG_GENESIS_HASH=$($spec.genesis_hash)" --env "CREG_STAKING_ADDR=$($spec.contracts.staking)" `
   --env CREG_CHAIN_SPEC_PATH=/spec/chain-spec.json --env CREG_CHAIN_SPEC_SIG_PATH=/spec/chain-spec.json.sig `
   --mount "type=bind,source=$specDirectory,target=/spec,readonly" --mount "type=bind,source=$keyFile,target=/publisher.key,readonly" `
   --mount "type=bind,source=$PrivateDirectory,target=/submission" $pin publish /submission/toidentifier-1.0.1.tgz `
   --manifest /submission/creg-manifest.json --key-file /publisher.key --publisher-address 0xa40D79527314efD4964D49c4a0673C87b612348B `
   --ipfs-url https://ipfs.testnet.cregnet.dev --offline /submission/signed-publish-toidentifier-r7.json
 if($LASTEXITCODE -ne 0){throw 'Real artifact pinning or signing failed.'}
}finally{
 $secret=$null; [Environment]::SetEnvironmentVariable('CREG_PUBLISHER_EVM_KEY',$savedKey,'Process')
}
$request=Get-Content $requestPath -Raw|ConvertFrom-Json
if($request.id.ecosystem -ne 'npm' -or $request.id.name -ne 'toidentifier' -or $request.id.version -ne '1.0.1' -or
 $request.content_hash -ne '186fcc77488de327daf911d362d4e773bab9909f1df2a5f0c20b875205b92e08' -or
 $request.publisher_chain_id -ne $spec.chain_id -or $request.publisher_authorization_version -ne 4 -or
 ($request.publisher_genesis_hash -replace '^0x','') -ne ($spec.genesis_hash -replace '^0x','') -or
 $request.publisher_staking_contract -ne $spec.contracts.staking){throw 'Signed request context does not match the real artifact and R7 spec.'}
Copy-Item $requestPath (Join-Path $EvidenceDirectory 'next-package/signed-publish-toidentifier-r7.json')
Write-Output 'Real npm artifact pinned and signed; authorization context verified. Not submitted.'
