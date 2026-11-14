# Stopped-node data backups before Phase C reset

Captured on 2026-09-25. Archives are stored outside the repository at
`C:\Users\samue\CREG-backups\2026-09-25-pre-phase-c`. They contain node data
and identity material and must remain private. No chain reset has occurred.

| Node | Archive | SHA-256 |
| --- | --- | --- |
| Laptop validator, Docker bind mount `C:\Users\samue\.creg-validator-validator-2\chain-data` | `validator-laptop-chain-data.zip` | `f52b983f46460e8fde64b4d68c41375318120720cbd294f7905185958abe36e5` |
| GCP east validator, bind mount `/home/samue/.creg-validator-validator-1/chain-data` | `validator-gcp-east-chain-data.tar.gz` | `72b8d093a3b093a48d69b154827c24f5e25160a1fec75cbae26e3b78e5a0e458` |
| Public observer, Docker volume `testnet_creg-node-data` | `observer-edge-chain-data.tar.gz` | `832eabbcc6fe9592b9f60d15266bd1921dfe64f24eb0ef853812fd4339dc1e12` |
| Previously stopped observer, bind mount `/opt/creg/observer-data` | `observer-old-chain-data.tar.gz` | `de92f4ab3bfa69a21770b4b85ee634aa5065a4a40f2527150628235f620207aa` |

The three active CREG containers were stopped individually during archive
creation, then restarted on their original data. Each reported `running
healthy` afterward. All four copied archives passed a full entry read or tar
listing check. The remote tar SHA-256 values match the local copies.

These are data backups, not Phase C acceptance evidence. The divergent block
responses and logs remain in the adjacent evidence files.
