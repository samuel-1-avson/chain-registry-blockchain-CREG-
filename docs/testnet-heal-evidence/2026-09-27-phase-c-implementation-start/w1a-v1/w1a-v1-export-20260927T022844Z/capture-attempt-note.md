# Capture Attempt Note

The consistent `chain.rocksdb` copy was completed while the V1 container was stopped, and V1 was restarted with its h12 tip unchanged. The first offline inspector invocation inherited the image's normal `creg-node` entrypoint, which attempted scanner initialization and exited before writing diagnostics or capture metadata.

This directory is retained as an incomplete attempt for auditability. It is excluded from W1a findings. The corrected capture uses `/app/creg_recovery_inspect` as the container entrypoint and produces a separate complete evidence directory.
