# Preflight Capture Attempt

At 2026-09-27 02:49 UTC, the read-only fleet collector queried all eight real nodes. It printed `ready=True` and `recovery_clear=True` for V1–V4 and O1–O4, but could not save the JSON because the new output directory did not yet exist. No node or cloud resource was changed by this attempt.

The collector itself requires its evidence parent directory to exist. The directory has now been created; the capture is being repeated to preserve the complete per-node response before rollout.
