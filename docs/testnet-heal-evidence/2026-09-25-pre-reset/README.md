# Live CREG evidence before Phase C reset

Captured on 2026-09-25 before any Phase C image deployment or chain reset.
These files are responses from real nodes and filtered real container logs;
they are not simulated acceptance evidence. `SHA256SUMS.txt` records the
SHA-256 digest of each captured data file.

| Node/source | Captured state | Files |
| --- | --- | --- |
| Laptop validator (`creg-validator-validator-2`) | h1 `916e01fde31fcc16ba5cb6bdaa69f8b95c97e1d428a7b5105fa682febf15fc2d` | `validator-laptop-*` |
| GCP validator (`creg-validator-1`, `us-east1-b`) | h2 `832ee073abc136a80725b880f28f2e1f96d475ee022f84d5ef61b95d0f9bf22b` | `validator-gcp-east-*` |
| Public observer on `creg-testnet-vm` | h0 `068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53` | `observer-edge-*` |
| GCP project inventory | VM names, zones, machine types, status, IPs | `gcp-hosts.csv` |

The two validator h1 responses carry the same block hash and two PBFT
signatures. The GCP validator's h2 response has two signatures; the laptop
and observer have not applied that block. Event files contain the last 160
matching log lines from the preceding 24 hours, selected for PBFT, publish,
revoke, sandbox, and sync terms. The files do not replace a stopped-node
data-volume backup, which must be taken before a coordinated reset.

The existing h1 publish contains the `SB010` degraded sandbox finding. It is
historical failure evidence and cannot count toward the required real-sandbox
four-validator acceptance run.

Stopped-node backup sources identified for the coordinated reset: laptop
validator bind mount `C:\Users\samue\.creg-validator-validator-2\chain-data`;
GCP east validator bind mount
`/home/samue/.creg-validator-validator-1/chain-data` on `creg-validator-1`;
public observer Docker volume `testnet_creg-node-data` on `creg-testnet-vm`.
The stopped-node archives and hashes are recorded in `BACKUP-MANIFEST.md`.
They were copied outside the repository and verified before any chain reset.
