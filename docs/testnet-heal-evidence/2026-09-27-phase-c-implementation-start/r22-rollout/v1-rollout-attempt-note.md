# V1 Rollout Attempt

The first invocation of the R22 local rollout helper exited in its read-only preservation-name preflight. PowerShell returned a null value when no prior preservation container existed, and the helper called `.Trim()` on it. The failure occurred before stopping or renaming the R21 V1 container. The helper now converts the container-list output to a string before checking it. The retry must still verify the current V1 state and exact R22 image digest before mutation.
