"""
Dataset curation and preprocessing for the CodeBERT malware classifier.

Supports labeled datasets from:
- npm audit advisories (JSON exports)
- PyPI security disclosures (JSON/CSV)
- Public datasets: MalOSS, SocketSecurity database
- Synthetic benign samples from top trusted packages
"""

import os
import json
import csv
import random
from typing import List, Dict, Tuple, Optional
from pathlib import Path

from datasets import Dataset as HFDataset, DatasetDict
from transformers import PreTrainedTokenizer


MAX_LENGTH = 512


def load_maloss_dataset(maloss_dir: str) -> List[Dict[str, any]]:
    """Load MalOSS dataset from directory of malicious packages."""
    examples = []
    root = Path(maloss_dir)
    if not root.exists():
        return examples

    for pkg_dir in root.iterdir():
        if not pkg_dir.is_dir():
            continue
        for code_file in pkg_dir.rglob("*"):
            if code_file.is_file() and code_file.suffix in {".js", "\u200b.py", ".ts", ".mjs", ".cjs"}:
                try:
                    code = code_file.read_text(encoding="utf-8", errors="ignore")
                    if len(code) >= 20:
                        examples.append({"code": code, "label": 1, "source": "maloss", "path": str(code_file)})
                except Exception:
                    pass
    return examples


def load_socketsecurity_dataset(socket_file: str) -> List[Dict[str, any]]:
    """Load SocketSecurity flagged packages from JSONL/CSV."""
    examples = []
    path = Path(socket_file)
    if not path.exists():
        return examples

    if path.suffix == ".csv":
        with open(path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                code = row.get("code", "")
                if code and len(code) >= 20:
                    examples.append({"code": code, "label": 1, "source": "socketsecurity", "path": row.get("path", "")})
    else:
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                try:
                    obj = json.loads(line)
                    code = obj.get("code", "")
                    if code and len(code) >= 20:
                        examples.append({"code": code, "label": 1, "source": "socketsecurity", "path": obj.get("path", "")})
                except json.JSONDecodeError:
                    continue
    return examples


def load_npm_advisories(advisory_file: str) -> List[Dict[str, any]]:
    """Load npm audit advisory code samples from JSON."""
    examples = []
    path = Path(advisory_file)
    if not path.exists():
        return examples

    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)

    for entry in data:
        code = entry.get("code", "")
        if code and len(code) >= 20:
            examples.append({"code": code, "label": 1, "source": "npm_advisory", "path": entry.get("path", "")})
    return examples


def load_pypi_disclosures(disclosure_file: str) -> List[Dict[str, any]]:
    """Load PyPI security disclosure code samples from JSON/CSV."""
    examples = []
    path = Path(disclosure_file)
    if not path.exists():
        return examples

    if path.suffix == ".csv":
        with open(path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                code = row.get("code", "")
                if code and len(code) >= 20:
                    examples.append({"code": code, "label": 1, "source": "pypi_disclosure", "path": row.get("path", "")})
    else:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        for entry in data:
            code = entry.get("code", "")
            if code and len(code) >= 20:
                examples.append({"code": code, "label": 1, "source": "pypi_disclosure", "path": entry.get("path", "")})
    return examples


def load_benign_github_dataset(language: str = "JavaScript", max_samples: int = 10_000) -> List[Dict[str, any]]:
    """
    Load a placeholder benign dataset from `codeparrot/github-code`.
    In production, replace with curated top-10k trusted npm/PyPI packages.
    """
    from datasets import load_dataset
    examples = []
    try:
        ds = load_dataset("codeparrot/github-code", language, split="train", streaming=True)
        for i, row in enumerate(ds):
            if i >= max_samples:
                break
            code = row.get("code", "")
            if code and len(code) >= 20:
                examples.append({"code": code, "label": 0, "source": "github_benign", "path": row.get("path", "")})
    except Exception as e:
        print(f"Failed to load github-code dataset: {e}")
    return examples


def balance_dataset(examples: List[Dict[str, any]], seed: int = 42) -> List[Dict[str, any]]:
    """Down-sample the majority class to balance labels."""
    random.seed(seed)
    malicious = [ex for ex in examples if ex["label"] == 1]
    benign = [ex for ex in examples if ex["label"] == 0]

    target = min(len(malicious), len(benign))
    malicious = random.sample(malicious, target) if len(malicious) > target else malicious
    benign = random.sample(benign, target) if len(benign) > target else benign

    combined = malicious + benign
    random.shuffle(combined)
    return combined


def build_dataset(
    maloss_dir: Optional[str] = None,
    socket_file: Optional[str] = None,
    npm_file: Optional[str] = None,
    pypi_file: Optional[str] = None,
    benign_samples: Optional[List[Dict[str, any]]] = None,
    max_benign: int = 10_000,
    test_size: float = 0.2,
    seed: int = 42,
) -> DatasetDict:
    """
    Build a balanced HuggingFace DatasetDict from all available sources.
    """
    examples = []

    if maloss_dir:
        examples.extend(load_maloss_dataset(maloss_dir))
    if socket_file:
        examples.extend(load_socketsecurity_dataset(socket_file))
    if npm_file:
        examples.extend(load_npm_advisories(npm_file))
    if pypi_file:
        examples.extend(load_pypi_disclosures(pypi_file))

    if benign_samples is None:
        benign_samples = load_benign_github_dataset(max_samples=max_benign)
    examples.extend(benign_samples)

    if not examples:
        raise ValueError("No training examples loaded. Provide at least one data source.")

    balanced = balance_dataset(examples, seed=seed)
    hf_ds = HFDataset.from_list(balanced)
    splits = hf_ds.train_test_split(test_size=test_size, seed=seed)
    return DatasetDict({"train": splits["train"], "validation": splits["test"]})


def tokenize_dataset(
    dataset: DatasetDict,
    tokenizer: PreTrainedTokenizer,
    max_length: int = MAX_LENGTH,
    text_column: str = "code",
) -> DatasetDict:
    """Tokenize a DatasetDict for CodeBERT training."""

    def _tokenize(examples):
        return tokenizer(
            examples[text_column],
            padding="max_length",
            truncation=True,
            max_length=max_length,
        )

    return dataset.map(_tokenize, batched=True, remove_columns=[text_column])
