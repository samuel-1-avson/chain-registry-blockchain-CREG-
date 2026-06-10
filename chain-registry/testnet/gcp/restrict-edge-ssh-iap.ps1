# Restrict edge VM SSH to IAP (35.235.240.0/20). Keeps gcloud compute ssh --tunnel-through-iap working.
param(
    [string]$ProjectId = "gen-lang-client-0022105784",
    [switch]$Apply
)
$ErrorActionPreference = "Stop"
$iapRange = "35.235.240.0/20"
$tag = "creg-testnet"
$iapRule = "creg-testnet-allow-iap-ssh"
$openRule = "creg-testnet-allow-ssh"
Write-Host "Project: $ProjectId"
if (-not $Apply) {
    Write-Host "Dry run. Re-run with -Apply to create IAP rule and narrow $openRule."
    exit 0
}
$null = gcloud compute firewall-rules describe $iapRule --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute firewall-rules create $iapRule --project=$ProjectId --direction=INGRESS --action=ALLOW --rules=tcp:22 --source-ranges=$iapRange --target-tags=$tag --description="SSH to edge VM via IAP only"
} else { Write-Host "$iapRule already exists" }
gcloud compute firewall-rules update $openRule --project=$ProjectId --source-ranges=$iapRange --description="SSH restricted to IAP (was 0.0.0.0/0)"
Write-Host "Done. Use: gcloud compute ssh creg-testnet-vm --tunnel-through-iap --project=$ProjectId --zone=us-central1-a"
