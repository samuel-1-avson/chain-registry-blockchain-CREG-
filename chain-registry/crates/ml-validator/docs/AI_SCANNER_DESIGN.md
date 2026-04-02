# AI Deep Learning Malware Scanner Design

> Phase 2 Security Improvement — Impact: 9/10  
> Status: Design Approved | Implementation: In Progress

## Overview

The AI Deep Learning Malware Scanner moves beyond the current rule-based / ONNX pattern classifier to **semantic code understanding**. It uses a fine-tuned **CodeBERT** model to classify package source code as malicious or benign at the AST-token level, then surfaces attention weights to highlight suspicious code regions.

---

## 1. Model Architecture

### Base Model
- **microsoft/codebert-base** (125 M parameters)
- Pre-trained on code from GitHub across six languages
- Strong performance on code-understanding tasks (code search, defect detection, clone detection)

### Fine-Tuning Objective
- **Binary classification**: `malicious` vs `benign`
- Input: tokenized source code (AST-level, max 512 tokens)
- Output: single logits → sigmoid → `malicious_probability` (0.0–1.0)

### Head Architecture
```
CodeBERT Encoder (125 M)
    ↓
[CLS] pooled output  →  Dropout(0.1)  →  Linear(768 → 1)
    ↓
                 Sigmoid
    ↓
         malicious_probability
```

### Attention Weights
- Extract last-layer attention from `[CLS]` token to all input tokens.
- Map token spans back to source lines to produce a **suspiciousness heatmap**.
- This allows validators (and later, publishers appealing a rejection) to see *which* functions or lines triggered the model.

---

## 2. Training Data Sources

| Source | Description | Label |
|--------|-------------|-------|
| **MalOSS Dataset** | Real-world malicious npm/PyPI packages | malicious |
| **SocketSecurity Reports** | Known malicious open-source packages | malicious |
| **PyPI Security Advisories** | Officially flagged packages | malicious |
| **npm Security Advisories** | GitHub/npm reported malicious packages | malicious |
| **GitHub Code** (`codeparrot/github-code`) | Random benign repositories | benign |
| **Top-10k npm/PyPI packages** | Highly trusted, widely-used packages | benign |

### Data Augmentation
- **Code obfuscation simulation**: minification, variable renaming, string splitting
- **Language mixing**: JavaScript ↔ TypeScript, Python minor version syntax differences
- **Truncation strategies**: head-only, tail-only, sliding window over 512 tokens

### Label Balance
- Target ratio: 1:5 malicious:benign (oversample malicious via duplication + augmentation)
- Stratified train/val/test split: 80/10/10

---

## 3. Feature Extraction Pipeline

```
Tarball (.tar.gz)
    ↓
Extract source files (*.js, *.ts, *.py, *.rs, *.rb, *.java)
    ↓
Merge into a single "document" per package
    ↓
Tree-sitter AST parse (optional, for future structural features)
    ↓
CodeBERT Tokenizer (WordPiece, vocab size 50,000)
    ↓
Truncate / pad to 512 tokens
    ↓
Input IDs + Attention Mask
    ↓
ONNX Runtime inference (`ort` crate)
    ↓
malicious_probability + attention_weights
```

### Tokenization Details
- Tokenizer: `microsoft/codebert-base` tokenizer (HuggingFace `tokenizers` / `transformers`)
- Rust side: either pre-export the tokenizer JSON and load with the `tokenizers` crate, or tokenize offline and store fixed-length vectors.
- **Short-term**: Python training pipeline exports both `model.onnx` and `tokenizer.json`.
- **Long-term**: Rust-native tokenization to avoid Python dependency at inference time.

---

## 4. ONNX Export & Rust Inference Integration

### Export (Python)
```python
from transformers.onnx import export
from transformers import AutoModelForSequenceClassification, AutoTokenizer

export(
    model=AutoModelForSequenceClassification.from_pretrained("./fine_tuned_codebert"),
    tokenizer=AutoTokenizer.from_pretrained("./fine_tuned_codebert"),
    output="malware_classifier.onnx",
    opset=14,
)
```

### Inference (Rust — `ort` crate)
```rust
use ort::{Environment, SessionBuilder, Value};
use ndarray::{Array1, Array2};

let session = SessionBuilder::new(&env)?
    .with_model_from_file("models/malware_classifier.onnx")?;

let input_ids = Array2::<i64>::from_shape_vec((1, 512), token_ids)?;
let outputs = session.run(vec![
    Value::from_array(session.allocator(), &input_ids)?
])?;

let prob = outputs[0].try_extract::<f32>()?[[0, 0]];
```

### Model Artifact Layout
```
chain-registry/models/
├── malware_classifier.onnx      # ONNX graph
├── tokenizer.json               # HuggingFace tokenizer config
└── malware_classifier_config.json  # threshold, version, training metadata
```

---

## 5. Thresholds & Decision Logic

| Probability | Action | Finding Severity |
|-------------|--------|------------------|
| `< 0.30` | Safe — no finding | — |
| `0.30 – 0.60` | Suspicious — add low-severity finding | Medium |
| `0.60 – 0.85` | Likely malicious — add high-severity finding, increase threat score | High |
| `≥ 0.85` | Confirmed malicious — critical finding, can trigger rejection | Critical |

### Integration with Existing Validator Pipeline
1. Existing `ml_validator::predict()` (rule-based / light ONNX) runs first.
2. **`ml_validator::deep_scan()` runs second** on the full tarball bytes.
3. The validator aggregates:
   - Static analysis findings
   - Deep-scan findings (`DS001`–`DS003`)
   - Publisher reputation
4. Final decision in `validator::final_decision()` considers deep-scan critical findings as **blocking** unless overridden by AAA (Automated AI Auditor) appeal.

### Appeal / Explainability
- Attention weights are serialized into the `ValidationReport` as `DeepScanResult.attention_regions`.
- During an appeal, the AAA service can re-run the same ONNX model and verify the attention heatmap cryptographically (via ZK proof or signed attestation in future phases).

---

## 6. Phase 2 Implementation Checklist

- [x] Design document (`AI_SCANNER_DESIGN.md`)
- [x] Rust stub: `deep_scan.rs` with `DeepScanResult` struct
- [x] Expose `deep_scan` from `ml-validator/src/lib.rs`
- [x] Wire into `validator/src/static_analysis.rs`
- [x] Python training pipeline stub
- [ ] Collect & label real malicious dataset
- [ ] Fine-tune CodeBERT on labeled data
- [ ] Export ONNX and validate with `ort`
- [ ] Optimize Rust tokenizer path (avoid Python dependency at runtime)
- [ ] ZK proof of ONNX inference (Phase 3)

---

## 7. Security Considerations

- **Model poisoning**: training data must be reviewed; supply-chain of base model weights verified via SHA-256.
- **Adversarial examples**: attackers may craft code that looks benign to the model. Mitigation: ensemble with rule-based validator and sandbox.
- **Privacy**: benign packages from private registries should not be logged to external training pipelines.
- **Availability**: if ONNX model is missing, `deep_scan()` gracefully degrades to a mock/placeholder result and logs a warning — **never blocks** the pipeline due to missing ML artifacts.
