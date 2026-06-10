param(
    [string]$EnvFile = ""
)

$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $EnvFile) {
    $EnvFile = Join-Path $gcpDir "hosting.env"
}

$defaults = @{
    GCP_PROJECT         = ""
    GCP_REGION          = "us-central1"
    GCP_ZONE            = "us-central1-a"
    GCP_VM_NAME         = "creg-testnet-vm"
    GCP_MACHINE_TYPE    = "e2-standard-4"
    GCP_BOOT_DISK_GB    = "100"
    GCP_STATIC_IP_NAME  = "creg-testnet-ip"
    PARENT_DOMAIN       = "cregnet.dev"
    BASE_DOMAIN         = "testnet.cregnet.dev"
    ACME_EMAIL          = ""
    GITHUB_REPO         = "samuel-1-avson/chain-registry-blockchain-CREG-"
    GITHUB_BRANCH       = "main"
    CF_API_TOKEN        = ""
}

$config = @{} + $defaults

if (Test-Path $EnvFile) {
    Get-Content $EnvFile | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        if ($line -match '^\s*([A-Za-z0-9_]+)\s*=\s*(.*)$') {
            $config[$Matches[1]] = $Matches[2].Trim('"').Trim("'")
        }
    }
}

if (-not $config.GCP_PROJECT) {
    $proj = (gcloud config get-value project 2>$null)
    if ($proj -and $proj -ne "(unset)") { $config.GCP_PROJECT = $proj.Trim() }
}

$resolved = $EnvFile
if (Test-Path -LiteralPath $EnvFile) {
    $resolved = (Resolve-Path -LiteralPath $EnvFile).Path
}
$config['ENV_FILE'] = $resolved

return $config
