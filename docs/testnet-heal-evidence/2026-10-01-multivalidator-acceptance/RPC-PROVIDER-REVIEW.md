# Sepolia RPC provider review

## Decision

Do not use anonymous public endpoints as the three-provider quorum configuration for R7. The validators repeatedly read Sepolia block headers and staking logs; public endpoints with small shared quotas or best-effort availability are not dependable inputs for that workload. Keep the existing PublicNode endpoint as provider one and configure two dedicated provider endpoints from distinct hosts. Store their full URLs, including credentials, in GCP Secret Manager and pass only secret resource names through the deployment workflow.

## Live probes

Read-only JSON-RPC `eth_chainId` probes were made during this review. PublicNode and Tenderly's generic Sepolia gateway returned Sepolia chain ID `0xaa36a7`. The documented 1RPC public endpoint timed out during the current probe. Earlier, a different 1RPC public URL and Tenderly returned the same Sepolia block height as PublicNode, but that point-in-time agreement does not establish sustained availability or sufficient quota.

Other anonymously accessible candidates were unsuitable in the probe: dRPC reported Sepolia unavailable on its free plan, Blast API reported that the service was no longer available, Ankr required an API key, and the BlockPI endpoint returned HTTP 521. These are observations from one probe, not guarantees about provider status.

## Provider constraints

- 1RPC's official site lists its Public tier at 200 requests/day with best-effort uptime. Its docs identify a public Sepolia endpoint. That quota is too low for the node sync workload. [1RPC plans](https://www.1rpc.io/) · [1RPC supported networks](https://docs.1rpc.io/using-the-web3-api/networks)
- Tenderly's documentation describes an account-specific gateway URL containing an access token and account usage tracking. A generic gateway response is not evidence of a dedicated production endpoint or an acceptable quota. Store a dedicated URL as a secret if Tenderly is selected. [Tenderly Web3 Gateway setup](https://tenderly.co/blog/how-to-send-transactions-with-web3-gateway/) · [Tenderly usage and rate-limiting](https://tenderly.co/blog/getting-started-with-web3-gateway/)

## Acceptance requirements

1. Each endpoint must resolve to a distinct provider hostname after URL parsing; duplicate paths or credentials on one hostname do not count as separate providers.
2. Verify all three endpoints report Sepolia chain ID `11155111`, comparable latest heights, and matching block hashes for the sync cursor and staking-log ranges.
3. Confirm each provider's current quota supports the validator poll and log-backfill workload. A chain-ID request alone is not readiness evidence.
4. Keep API URLs and access tokens out of Git, logs, evidence JSON, and chat. Record only secret names and sanitized hostnames.
5. Reconfigure validators one at a time, preserve the current node data and signing journal, and capture fresh sync status after each restart. Do not submit the acceptance package until all four validator sync cursors are current and quorum-backed.
