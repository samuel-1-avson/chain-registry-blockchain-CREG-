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
    GCP_TESTNET_TAG     = "creg-testnet"
    GCP_SEPOLIA_GETH_VM_NAME = "creg-sepolia-geth-vm"
    GCP_SEPOLIA_GETH_TAG = "creg-sepolia-geth"
    GCP_SEPOLIA_GETH_INTERNAL_IP_NAME = "creg-sepolia-geth-internal-ip"
    GCP_SEPOLIA_GETH_MACHINE_TYPE = "e2-standard-2"
    GCP_SEPOLIA_GETH_DISK_GB = "100"
    GCP_VALIDATOR_VM_NAME = "creg-validator-vm"
    GCP_VALIDATOR_TAG = "creg-validators"
    GCP_VALIDATOR_INTERNAL_IP_NAME = "creg-validator-internal-ip"
    GCP_VALIDATOR_MACHINE_TYPE = "e2-standard-8"
    GCP_VALIDATOR_DISK_GB = "50"
    PARENT_DOMAIN       = "cregnet.dev"
    BASE_DOMAIN         = "testnet.cregnet.dev"
    ACME_EMAIL          = ""
    GITHUB_REPO         = "samuel-1-avson/chain-registry-blockchain-CREG-"
    GITHUB_BRANCH       = "main"
    CF_API_TOKEN        = ""
    GCP_WAITLIST_PROJECT = "gen-lang-client-0098858574"
    GCP_COST_LABEL_ENV  = "public-alpha"
    GCP_COST_LABEL_OWNER = "creg"
    GCP_COST_LABEL_COST_CENTER = "testnet"
    GCP_BUDGET_TESTNET_USD = "300"
    GCP_BUDGET_WAITLIST_USD = "25"
    GCP_BUDGET_ALERT_EMAIL = ""
    GCP_ALERT_SLACK_WEBHOOK_URL = ""
    GCP_ALERT_PAGERDUTY_ROUTING_KEY = ""
    GCP_ALERT_NTFY_TOPIC = ""
    GCP_ALERT_NTFY_SERVER = "https://ntfy.sh"
    GCP_ALERT_NTFY_TOPIC_SECRET = "creg-testnet-alert-ntfy-topic"
    GCP_ALERT_SLACK_WEBHOOK_SECRET = "creg-testnet-alert-slack-webhook"
    GCP_ALERT_WEBHOOK_SECRET = "creg-testnet-alert-webhook"
    GCP_ALERT_PAGERDUTY_SECRET = "creg-testnet-alert-pagerduty-key"
    GCP_ALERT_SMTP_PASSWORD_SECRET = "creg-testnet-alert-smtp-password"
    GCP_ALERT_WEBHOOK_URL = ""
    GCP_ALERT_EMAIL_TO = ""
    GCP_ALERT_SMTP_HOST = ""
    GCP_ALERT_SMTP_USER = ""
    GCP_ALERT_SMTP_PASSWORD = ""
    GCP_STATIC_BUCKET   = ""
    GCP_OBSERVER_MIG_NAME = "creg-observer-pool"
    GCP_OBSERVER_MACHINE_TYPE = "e2-medium"
    GCP_OBSERVER_DISK_GB = "30"
    GCP_OBSERVER_TAG = "creg-observers"
    GCP_OBSERVER_ILB_NAME = "creg-observer-api-ilb"
    GCP_OBSERVER_ILB_IP_NAME = "creg-observer-api-ilb-ip"
    GCP_HUB_API_SERVICE = "creg-hub-api"
    GCP_ARTIFACT_REPO = "creg-testnet"
    GCP_ARMOR_POLICY_NAME = "creg-testnet-edge"
    GCP_EDGE_BACKEND_NAME = "creg-edge-api-backend"
    GCP_API_LB_IP_NAME = "creg-api-lb-ip"
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
