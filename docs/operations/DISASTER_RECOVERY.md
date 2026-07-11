# Disaster recovery

Back up canonical event history, log tree heads, evidence manifests, object
metadata, policy/profile schemas, deployment manifests, encrypted secret
references, and database snapshots across regions/domains. Do not back up raw
secrets in plaintext.

Recovery must rebuild PostgreSQL/search/explorer views from canonical events,
verify object digests, restore witness consistency, force client freshness, and
record RTO/RPO evidence. Exercise total database loss, one object provider
loss, one witness loss, one region loss, and key compromise before production.
