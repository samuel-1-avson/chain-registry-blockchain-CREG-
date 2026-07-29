# -*- mode: python ; coding: utf-8 -*-
"""Auditable PyInstaller one-file recipe for the production verifier."""

from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo


project_root = Path(SPECPATH).resolve()


def canonicalize_base_library(analysis):
    """Remove filesystem-order variance from PyInstaller's stdlib archive."""

    matches = [
        Path(source)
        for destination, source, kind in analysis.datas
        if destination == "base_library.zip" and kind == "DATA"
    ]
    if len(matches) != 1:
        raise RuntimeError(f"expected one base_library.zip, found {len(matches)}")
    archive = matches[0]
    replacement = archive.with_suffix(".canonical.zip")
    with ZipFile(archive, "r") as source:
        members = [
            (member.filename, source.read(member))
            for member in sorted(source.infolist(), key=lambda value: value.filename)
        ]
    with ZipFile(replacement, "w") as target:
        for filename, contents in members:
            member = ZipInfo(filename, date_time=(1980, 1, 1, 0, 0, 0))
            member.compress_type = ZIP_DEFLATED
            member.create_system = 3
            member.external_attr = 0o100644 << 16
            target.writestr(member, contents, compresslevel=9)
    replacement.replace(archive)


analysis = Analysis(
    [str(project_root / "packaging" / "entrypoint.py")],
    pathex=[str(project_root / "src")],
    binaries=[],
    datas=[],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
canonicalize_base_library(analysis)
python_archive = PYZ(analysis.pure)

executable = EXE(
    python_archive,
    analysis.scripts,
    analysis.binaries,
    analysis.datas,
    [],
    name="creg-pypi-authority-verifier",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
