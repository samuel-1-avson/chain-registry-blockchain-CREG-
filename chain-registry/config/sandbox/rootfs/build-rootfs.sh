#!/usr/bin/env bash
# Build minimal rootfs images for each ecosystem, export their filesystem
# layers into config/sandbox/rootfs/{ecosystem}/, and emit SHA-256 tree
# digests in config/sandbox/rootfs/rootfs-digests.json (`VNA-14`).
#
# Usage:
#   cd chain-registry/
#   bash config/sandbox/rootfs/build-rootfs.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOTFS_DIR="$SCRIPT_DIR"

ECOSYSTEMS=("npm" "pip" "cargo" "rubygems" "maven")

for eco in "${ECOSYSTEMS[@]}"; do
    echo "=== Building rootfs for: $eco ==="
    DOCKERFILE="$ROOTFS_DIR/Dockerfile.$eco"
    IMAGE_TAG="creg-rootfs-$eco:latest"
    DEST="$ROOTFS_DIR/$eco"

    if [ ! -f "$DOCKERFILE" ]; then
        echo "  SKIP: No Dockerfile found at $DOCKERFILE"
        continue
    fi

    docker build -t "$IMAGE_TAG" -f "$DOCKERFILE" "$ROOTFS_DIR"

    rm -rf "$DEST"
    mkdir -p "$DEST"

    CONTAINER_ID=$(docker create "$IMAGE_TAG")
    docker export "$CONTAINER_ID" | tar -xf - -C "$DEST"
    docker rm "$CONTAINER_ID" >/dev/null

    echo "  Rootfs for $eco exported to $DEST ($(du -sh "$DEST" | awk '{print $1}'))"
done

echo "=== Computing deterministic CREG-ROOTFS-TREE-V1 digests ==="
ROOTFS_DIR="$ROOTFS_DIR" python3 - <<'PY'
import hashlib, json, os, stat

root = os.environ["ROOTFS_DIR"]
ecosystems = ["npm", "pip", "cargo", "rubygems", "maven"]
manifest = {"schema_version": 1, "ecosystems": {}}

for eco in ecosystems:
    eco_dir = os.path.join(root, eco)
    if not os.path.isdir(eco_dir):
        continue
    entries = []
    for cur, dirs, files in os.walk(eco_dir, followlinks=False):
        rel_cur = os.path.relpath(cur, eco_dir).replace("\\", "/")
        for d in dirs:
            p = os.path.join(cur, d)
            rel = d if rel_cur == "." else f"{rel_cur}/{d}"
            entries.append((rel, p))
        for f in files:
            p = os.path.join(cur, f)
            rel = f if rel_cur == "." else f"{rel_cur}/{f}"
            entries.append((rel, p))
    entries.sort(key=lambda x: x[0])

    h = hashlib.sha256()
    h.update(b"CREG-ROOTFS-TREE-V1\n")
    reg_count = 0
    for rel, p in entries:
        st = os.lstat(p)
        if stat.S_ISLNK(st.st_mode):
            target = os.readlink(p).replace("\\", "/")
            h.update(f"l {rel} {target}\n".encode("utf-8"))
        elif stat.S_ISDIR(st.st_mode):
            h.update(f"d {rel}\n".encode("utf-8"))
        elif stat.S_ISREG(st.st_mode):
            reg_count += 1
            fh = hashlib.sha256()
            size = 0
            with open(p, "rb") as fp:
                while True:
                    chunk = fp.read(16384)
                    if not chunk:
                        break
                    size += len(chunk)
                    fh.update(chunk)
            h.update(f"f {rel} {size} {fh.hexdigest()}\n".encode("utf-8"))
    manifest["ecosystems"][eco] = {
        "sha256": h.hexdigest(),
        "file_count": reg_count,
        "base_image": f"creg-rootfs-{eco}:pinned",
    }

out_path = os.path.join(root, "rootfs-digests.json")
with open(out_path, "w", encoding="utf-8") as fp:
    json.dump(manifest, fp, indent=2, sort_keys=True)
    fp.write("\n")
print(f"Wrote {out_path}")
PY

echo ""
echo "All rootfs images built and digest-pinned in $ROOTFS_DIR/rootfs-digests.json."
