# CREG GCP cost estimate and implemented savings

**Captured:** 2 October 2026 (UTC)  
**Scope:** Live CREG infrastructure in projects `gen-lang-client-0022105784` and `fiffi-500411`, plus the associated Cloud Run project `gen-lang-client-0098858574`  
**Billing limitation:** No invoice or billing export was available, so dollar totals below are resource-based list-price estimates, not amounts charged to the shared billing account.

## Current monthly estimate

CREG currently has seven running Compute Engine VMs: four `e2-medium` instances and three `e2-small` observers. The four validators remain unchanged; all four geographically separate observer hosts remain in service. The estimate assumes 730 running hours per month.

| Cost item | Current estimate |
|---|---:|
| Four `e2-medium` plus three `e2-small` VMs | $142.86/month* |
| Attached persistent disks | $33.27/month |
| Six in-use VM public IPv4 addresses | $21.90/month |
| **Estimated CREG baseline** | **$198.03/month** |

\*The compute figure uses the previous fleet estimate of $181.82/month for seven `e2-medium` VMs, then subtracts an estimated $38.96/month for the three right-sized observers. The prior estimate does not include an exact per-zone rate split, so this blended result is approximate. Actual regional prices, sustained-use discounts, credits, billing currency, and negotiated rates can change it.

Cloud NAT is configured in `us-central1`; at the illustrative rate of one private VM and one public NAT IP, it adds about **$4.67/month**, plus **$0.045 per GiB processed**. Including only that illustrative base NAT charge gives about **$202.70/month**, before traffic, internet egress, logging, snapshots, backups, taxes, and other services. The associated billing account also contains unrelated projects, so its total bill is higher.

The estimate is not the GCP invoice. For an exact billed total or savings percentage, use Cloud Billing → Reports or a billing export and filter by the CREG projects and services. Credits and shared-account costs are not represented above.

## Implemented reduction

On 2 October 2026, the three non-voting observers below moved from `e2-medium` (4 GiB) to `e2-small` (2 GiB). O3 and O4 were changed one at a time; a refreshed live inventory also showed O2 at `e2-small`, and it passed the same health checks:

| VM | Region | Before | Now | Reserved IP |
|---|---|---|---|---|
| `creg-observer-vm` (O2) | `us-central1-a` | `e2-medium` | `e2-small` | Preserved |
| `creg-observer-3` (O3) | `us-west1-b` | `e2-medium` | `e2-small` | Preserved |
| `creg-observer-4` (O4) | `africa-south1-a` | `e2-medium` | `e2-small` | Preserved |

Each resized observer is running and passed its live readiness check. At the final capture, all eight nodes reported finalized height 20 with the same block hash; PBFT and recovery checks were clear. O2, O3, and O4 passed their observer checks. Their point-in-time container memory readings were approximately 87 MiB, 35 MiB, and 37 MiB, respectively, against about 1.92 GiB available to each container.

**Best current estimate: about 16.44% off the former $236.99/month CREG baseline**, or roughly **$38.96/month**, bringing the estimate to **$198.03/month** before NAT, egress, and other excluded services. This cuts the VM compute price for three observers by 50% each while keeping every validator and all four observer regions running. The percentage of the actual invoice can only be confirmed from billing data.

The full readiness gate still reports existing validator configuration warnings: each validator has only one configured L1 RPC provider, so the fleet-wide `ready` flag remains false even though all nodes shared a tip and PBFT/recovery checks were clear. This predates the resizing; I did not change validator configuration as part of cost work.

Fresh inventory is in `live-inventory.json`. The before, after-O3, after-O3/O4, and final health captures are `fleet-readiness-before.json`, `fleet-readiness-after-o3.json`, `fleet-readiness-after-rightsize.json`, and `fleet-readiness-final.json`.

## What I would do next

- Keep this right-sized fleet running while collecting a full billing month and workload data. Do not remove O3/O4: stopping them would take the deployment below its four-independent-observer operational target.
- Defer 1-year or 3-year commitments. The live quota check showed `COMMITMENTS` and `COMMITTED_E2_CPUS` quotas at zero, and Google documents these quotas as prerequisites for purchasing resource-based commitments. A commitment also creates a binding charge even if usage later falls. Revisit only after billing eligibility, sustained usage, and the monthly bill are confirmed.
- Leave validators at `e2-medium`; they are voting members, and consensus needs three of four. Do not use Spot for them.
- Keep Cloud Run services at zero minimum instances; the inventory showed zero configured minimums already. Check billed requests, builds, artifact storage, and databases if the CREG Functions project still has material spend.
- Review the stopped legacy VM's retained 40-GiB balanced disk before any deletion. It may cost around $4/month, but first confirm its data is no longer needed.

The earlier 50% scenario depended on removing two independent observers and a long commitment; it is not the right fit for the current operating target. An 80% reduction is not supported by the current continuously available four-validator design. The implemented ~16% reduction is reversible and preserves that design.

Official references: [E2 VM pricing](https://cloud.google.com/products/compute/pricing/general-purpose), [changing a VM machine type](https://docs.cloud.google.com/compute/docs/instances/changing-machine-type-of-stopped-instance), [resource-based commitment eligibility and billing](https://docs.cloud.google.com/compute/docs/instances/signing-up-committed-use-discounts), [Cloud Billing reports](https://docs.cloud.google.com/billing/docs/how-to/reports).
