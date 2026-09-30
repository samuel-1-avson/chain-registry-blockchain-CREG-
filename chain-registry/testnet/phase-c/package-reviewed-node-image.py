"""Preserve a real Docker image, plus a delta requiring its exact existing base."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile

image, base_image, directory, stem = sys.argv[1:]
destination = Path(directory)
destination.mkdir(parents=True, exist_ok=True)
full = destination / (stem + '.tar.gz')
delta = destination / (stem + '-delta.tar.gz')


def inspect(tag):
    return json.loads(subprocess.check_output(['docker', 'image', 'inspect', tag]))[0]


reviewed = inspect(image)
base = inspect(base_image)
base_count = len(base['RootFS']['Layers'])
if reviewed['RootFS']['Layers'][:base_count] != base['RootFS']['Layers']:
    raise RuntimeError('The reviewed image does not retain the exact required base layers')

raw_digest = hashlib.sha256()
process = subprocess.Popen(['docker', 'save', image], stdout=subprocess.PIPE)
with full.open('xb') as output, gzip.GzipFile(fileobj=output, mode='wb', compresslevel=1, mtime=0) as compressed:
    while chunk := process.stdout.read(8 * 1024 * 1024):
        raw_digest.update(chunk)
        compressed.write(chunk)
if process.wait() != 0:
    raise RuntimeError('Docker save failed; incomplete archive retained for diagnosis')

roundtrip = hashlib.sha256()
with gzip.open(full, 'rb') as source:
    while chunk := source.read(8 * 1024 * 1024):
        roundtrip.update(chunk)
if roundtrip.digest() != raw_digest.digest():
    raise RuntimeError('Archive roundtrip digest mismatch')

with tarfile.open(full, 'r:gz') as archive:
    entries = json.load(archive.extractfile('manifest.json'))
    if len(entries) != 1 or image not in entries[0]['RepoTags']:
        raise RuntimeError('Archive does not contain the single reviewed image')
    omitted = set(entries[0]['Layers'][:base_count])
    if len(omitted) != base_count:
        raise RuntimeError('Unexpected repeated base layers')
    with delta.open('xb') as output, gzip.GzipFile(fileobj=output, mode='wb', compresslevel=1, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w|') as reduced:
            for member in archive:
                if member.name in omitted:
                    continue
                reduced.addfile(member, archive.extractfile(member) if member.isfile() else None)


def sha256(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


print(json.dumps({
    'image_tag': image, 'image_id': reviewed['Id'], 'required_base_image': base_image,
    'required_base_image_id': base['Id'], 'base_layers_omitted': base_count,
    'archive_path': str(full), 'archive_bytes': full.stat().st_size,
    'archive_sha256': sha256(full), 'archive_raw_sha256': raw_digest.hexdigest(),
    'archive_roundtrip_verified': True, 'delta_archive_path': str(delta),
    'delta_archive_bytes': delta.stat().st_size, 'delta_archive_sha256': sha256(delta),
    'delta_archive_requires_exact_base_image': True,
    'delta_archive_base_image': base_image,
    'delta_archive_base_image_id': base['Id'],
}))
