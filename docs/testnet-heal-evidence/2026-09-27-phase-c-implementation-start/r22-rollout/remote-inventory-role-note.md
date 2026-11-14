# Remote Inventory Role Check

The read-only inventory script's first pass was marked failed because its `ROLE` placeholder replacement also rewrote the literal `CREG_NODE_ROLE` environment key before parsing it. A direct V2 read had already confirmed the expected validator role, and all nodes passed the live health-role check. No remote configuration was changed. The corrected script uses distinct placeholder tokens and retains the failed inventory outputs.
