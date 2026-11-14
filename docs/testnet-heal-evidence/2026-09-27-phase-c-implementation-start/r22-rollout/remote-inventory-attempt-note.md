# Remote Target Inventory Attempt

The initial read-only inventory reached O1 but ran as the SSH user, who cannot read the root-owned `node.env`. The command returned a permission error before it wrote any remote state and did not include environment values in its output. The corrected inventory runs the same restricted, read-only parser with `sudo`; it records only selected node settings, SHA-256 fingerprints and file-presence checks.
