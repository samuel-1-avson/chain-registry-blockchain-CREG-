# CREG Validator Console (Desktop GUI)

Native Tauri v2 + React/TypeScript operator desktop application for the CREG four-zone validator node (`creg-gateway`, `creg-consensusd`, `creg-sandboxd`, `creg-agent-runner`).

## Features
- **R7 operation:** **Zones & Runtime** lists running validators and inactive containers. Use **Manage existing validator** to open the wallet flow for a running node with its existing local key. Use **Enroll a new validator** for a new identity. Review the connected Sepolia wallet, node ID, and public key together before signing.
- **Node selection:** **Settings → Verified R7 node** remembers the selected endpoint for monitoring and evidence queries. The R7 table reports API reachability, block sync, peers, and Sepolia sync independently.
- **Runtime Controller:** Start, stop, inspect, and stream logs from either `chain-registry/docker-compose.validator-v2.yml` or a local native `creg-node-supervisor` / `creg-consensusd` process.
- **Four-Zone Diagnostics (`zone-ipc`):** Reuses `zone_ipc::OperatorIpcKeyBundleV1` (`ZoneId::Operator`) to probe Gateway, Consensus, Sandbox, and Agent Runner capabilities without ever exposing signing or API keys.
- **AI Config & Probing (`ai-config`):** Validates `validator-ai.toml` (SSRF URL policy, `env:/file:/vault:` secret references, model-family diversity) before atomic saves and triggers Zone 4 `ProbeProvider` checks.
- **Logs & Live SSE v2 Feed:** Real-time runtime logs plus `Last-Event-ID` / `SseEnvelopeV2` event stream.
- **Evidence & Quarantine Inspector (`registry-view`):** Inspects 72h fail-closed quarantines (server-clock countdown), Package v4 status timelines, Class B `TriggerWitnessV1` proofs (`creg evidence verify-witness` CLI command), and epoch committee diversity.

## Development & Build (Windows PowerShell)

The desktop management action looks for an existing key at
`%USERPROFILE%\.creg-validator-<node-id>\validator.key`. For a node with a
different key location, run `creg validator onboard --manage-existing --node-id <node-id> --local-node-url <url> --key-file <path>`.

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
