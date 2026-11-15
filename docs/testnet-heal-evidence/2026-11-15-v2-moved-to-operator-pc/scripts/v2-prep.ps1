# Prepare this PC for validator-2. Changes nothing in the cloud.
$ErrorActionPreference = 'Continue'
$dir = 'C:\Users\samue\.cargo-targets\phase0-logs\v2move'
$out = Join-Path $dir 'v2-prep.log'
$log = New-Object System.Collections.Generic.List[string]
function Add([string]$m) { $log.Add($m); $log | Set-Content $out }
function Fail([string]$m) { Add "FAILED: $m"; exit 1 }
$ref = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$binary = 'c7ef1d3e8bb9b34e2e0e4e255bf47f751ecaada4d0617f9ac5a4dbedb262a5b1'
$root = 'C:\Users\samue\.creg-phase-c\r7\v2'
$private = 'C:\Users\samue\.creg-phase-c\r7\v2-private'

foreach ($p in @($root, $private)) { if (Test-Path -LiteralPath $p) { Fail "$p already exists" } }
if (docker ps -a --format '{{.Names}}' | Where-Object { $_ -eq 'creg-phase-c-r7-v2' }) { Fail 'a local creg-phase-c-r7-v2 container already exists' }

$cfg = 'C:\Users\samue\.cargo-targets\canary-r34-stage\docker-cfg-v2'
New-Item -ItemType Directory -Force $cfg | Out-Null
[IO.File]::WriteAllText((Join-Path $cfg 'config.json'), '{"credHelpers":{"us-central1-docker.pkg.dev":"gcloud"}}', (New-Object Text.UTF8Encoding($false)))
$env:DOCKER_HOST = 'npipe:////./pipe/dockerDesktopLinuxEngine'
$pull = docker --config $cfg pull $ref 2>&1
$code = $LASTEXITCODE
Remove-Item -Recurse -Force $cfg
Remove-Item Env:DOCKER_HOST
Add "pull exit: $code"
if ($code -ne 0) { Add (($pull | Select-Object -Last 3) -join ' | '); Fail 'docker pull' }
$id = (docker image inspect $ref --format '{{.Id}}' | Out-String).Trim()
Add "image id: $id"
$local = ((docker run --rm --entrypoint /usr/bin/sha256sum $ref /app/creg-node | Out-String).Trim() -split '\s+')[0]
Add "binary sha256: $local"
if ($local -ne $binary) { Fail 'binary differs from the one running on the VM' }
$ver = (docker run --rm $ref --version | Out-String).Trim()
Add "version: $ver"

New-Item -ItemType Directory -Path $root | Out-Null
New-Item -ItemType Directory -Path $private | Out-Null
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$acl = icacls.exe $private /inheritance:r /grant:r "*$($sid):(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' 2>&1
Add "acl exit: $LASTEXITCODE"
if ($LASTEXITCODE -ne 0) { Fail 'could not restrict the private folder' }
Add "acl: $((icacls.exe $private | Out-String).Trim() -replace '\s+', ' ')"
Add 'prep complete'
'finished'
