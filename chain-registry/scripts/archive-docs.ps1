# Script to organize docs/ directory:
# Archives completed milestone reports, historical design drafts, and intermediate plans into docs/archive/.
# Keeps active reference documentation, whitepaper, operational runbooks, and current guides in docs/.

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$chainRegistryDir = Split-Path -Parent $scriptDir
$repoRoot = Split-Path -Parent $chainRegistryDir
$docsDir = Join-Path $repoRoot "docs"
$archiveDir = Join-Path $docsDir "archive"

if (-not (Test-Path $archiveDir)) {
    New-Item -ItemType Directory -Path $archiveDir -Force | Out-Null
}

$filesToArchive = @(
    "ARENA_PROTOCOL_CHANGELOG.md",
    "BLOCKCHAIN_AND_DECENTRALIZATION_REPORT.md",
    "CREG_COMPREHENSIVE_AUDIT_2026-07-10.md",
    "CREG_COMPREHENSIVE_AUDIT_2026-07-10.mermaid-source.md",
    "CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md",
    "CROSS_CHAIN_TRANSPARENCY_REPORT.md",
    "DEVELOPER_REGISTRY_GATEWAY_REPORT.md",
    "DUAL_CONSENSUS_ENGINE_DEEP_DIVE_AND_RATING_REPORT.md",
    "GCP-BUDGET-ARCHITECTURE.md",
    "GCP-SEPOLIA-GETH-INTERNAL.md",
    "GCP-VALIDATOR-FLEET.md",
    "GCP_PHASE2_RUNBOOK.md",
    "GENESIS_ALPHA_COHORT_PLAYBOOK.md",
    "HYBRID-LOCAL-VALIDATORS.md",
    "L1_L2_HARDENING.md",
    "L2_PUBLIC_ALPHA_GATE_STATUS.md",
    "LAYER1_ENHANCEMENTS_COMPLETION_REPORT.md",
    "LAYER1_ENHANCEMENTS_IMPLEMENTATION_PLAN.md",
    "LAYER1_PARALLEL_ANALYSIS_AND_EVIDENCE_CONSENSUS_DEEP_DIVE.md",
    "LAYER2_PBFT_CONSENSUS_HARDENING_PLAN.md",
    "LAYER2_PBFT_CONSENSUS_HARDENING_REPORT.md",
    "MULTI_NODE_TESTNET_SIMULATION_REPORT.md",
    "NEXT_WORK.md",
    "P2P_NETWORK_HARDENING_PLAN.md",
    "P2P_NETWORK_HARDENING_REPORT.md",
    "PHASE2_PHASE3_IMPLEMENTATION_PLAN.md",
    "PUBLIC.md",
    "REMEDIATION_BACKLOG.md",
    "SEC-401-AUDIT-SCOPE.md",
    "SEC-401-VENDOR-OUTREACH.md",
    "SUBMISSION_LAYER_DEEP_DIVE_REPORT.md",
    "SUBMISSION_LAYER_IMPLEMENTATION_PLAN.md",
    "SUBMISSION_LAYER_IMPROVEMENT_AND_RATING_REPORT.md",
    "SUBMISSION_LAYER_MILESTONE_REPORT.md",
    "SUBMISSION_LAYER_PHASE1_COMPLETION_REPORT.md",
    "SUBMISSION_LAYER_PHASE2_COMPLETION_REPORT.md",
    "SUBMISSION_LAYER_UPDATED_RATING_REPORT.md",
    "TESTNET-HUB-DESIGN.md",
    "TESTNET_ORCHESTRATION_L1_AND_BENCHMARK_REPORT.md",
    "TESTNET_PHASE_SCOPE.md",
    "TESTNET_READINESS_REPORT.md",
    "TESTNET_SCALE_ARCHITECTURE.md",
    "TESTNET_SEPOLIA_RUNBOOK.md",
    "TESTNET_STATUS.md",
    "WAITLIST-ADMIN-MONITORING.md",
    "WAITLIST-ONLY-HOSTING.md",
    "WAITLIST_ARENA_MERGE_AUDIT.md",
    "WAITLIST_FIREBASE_AUDIT_CHECKLIST.md",
    "WAITLIST_FIREBASE_DEPLOY.md",
    "WEAKNESSES_GAPS_REMEDIATION_PLAN.md",
    "WHAT_TO_DO_NEXT.md",
    "WITNESS_OPERATOR_OUTREACH.md",
    "genesis-alpha-wave1-tracker.csv"
)

Write-Host "Organizing documentation in $docsDir..." -ForegroundColor Cyan

$archivedCount = 0
foreach ($file in $filesToArchive) {
    $srcPath = Join-Path $docsDir $file
    if (Test-Path $srcPath) {
        Move-Item -Path $srcPath -Destination $archiveDir -Force
        $archivedCount++
        Write-Host "  [ARCHIVED] $file" -ForegroundColor DarkGray
    }
}

Write-Host ""
Write-Host "Successfully archived $archivedCount files into docs/archive/." -ForegroundColor Green
