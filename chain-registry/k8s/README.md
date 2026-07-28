# Kubernetes deployment prerequisites

Repository manifests never contain operator, validator, database, or API
secrets. Create the required Secrets in the `chain-registry` namespace before
running `kubectl apply -k k8s`.

At minimum, the default validator deployment requires:

```text
kubectl -n chain-registry create secret generic creg-secrets \
  --from-literal=NODE1_VALIDATOR_KEY='<validator-1-private-key>' \
  --from-literal=CREG_OPERATOR_API_KEY='<high-entropy-server-side-secret>'

kubectl -n chain-registry create secret generic postgres-secrets \
  --from-literal=POSTGRES_PASSWORD='<high-entropy-database-password>'
```

If optional validator manifests are enabled, add the corresponding
`NODE2_VALIDATOR_KEY` through `NODE10_VALIDATOR_KEY` entries to `creg-secrets`.
Use the deployment's external secret manager or sealed-secret workflow for
real environments instead of placing literal values in shell history.

`CREG_OPERATOR_API_KEY` is the only credential accepted by the private API
ACL. `CREG_OPERATOR_PUBKEY` is public verification material and cannot unlock
private routes. Never expose the API key in a `VITE_*` variable, browser
bundle, public proxy configuration, URL, request body, ConfigMap, or committed
manifest.

All validator `secretKeyRef` entries are non-optional. A missing Secret or key
therefore prevents the Pod from starting instead of silently selecting a
default.

The base `creg-config` keeps `CREG_WEAK_SUBJECTIVITY_CHECKPOINT` empty and
`CREG_REQUIRE_CERTIFIED_COMMITTEES=false`. For a staged rollout, provision the
complete independently verified signed checkpoint through an
environment-specific ConfigMap/overlay, rebuild an observer canary, and compare
its certified history with independently controlled peers. Enable mandatory
mode only after the checkpoint and recovery rehearsal succeeds; never select a
checkpoint from gossip or one HTTP peer.
