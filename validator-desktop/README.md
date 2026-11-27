# CREG Validator Console (Desktop GUI)

Native Tauri v2 + React/TypeScript operator desktop application for the CREG four-zone validator node (`creg-gateway`, `creg-consensusd`, `creg-sandboxd`, `creg-agent-runner`).

## Features
- **R7 onboarding:** Open the local wallet flow from **Zones & Runtime**. An installed CREG CLI with the `validator onboard` command handles staking, dual-signed registration, and readiness. Connect an Ethereum browser wallet on Sepolia to approve the transaction and sign your identity.
- **Runtime Controller:** Start, stop, inspect, and stream logs from either `chain-registry/docker-compose.validator-v2.yml` or a local native `creg-node-supervisor` / `creg-consensusd` process.
- **Four-Zone Diagnostics (`zone-ipc`):** Reuses `zone_ipc::OperatorIpcKeyBundleV1` (`ZoneId::Operator`) to probe Gateway, Consensus, Sandbox, and Agent Runner capabilities without ever exposing signing or API keys.
- **AI Config & Probing (`ai-config`):** Validates `validator-ai.toml` (SSRF URL policy, `env:/file:/vault:` secret references, model-family diversity) before atomic saves and triggers Zone 4 `ProbeProvider` checks.
- **Logs & Live SSE v2 Feed:** Real-time runtime logs plus `Last-Event-ID` / `SseEnvelopeV2` event stream.
- **Evidence & Quarantine Inspector (`registry-view`):** Inspects 72h fail-closed quarantines (server-clock countdown), Package v4 status timelines, Class B `TriggerWitnessV1` proofs (`creg evidence verify-witness` CLI command), and epoch committee diversity.

## Development & Build (Windows PowerShell)

```powershell
cd validator-desktop
npm.cmd install
npm.cmd run build
npm.cmd run tauri:dev
```

To build a distributable installer:

```powershell
cd validator-desktop
npm.cmd run tauri:build
```
