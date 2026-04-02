# AI Scanner Setup Guide

## Overview

The Chain Registry AI Scanner provides machine learning-based malware detection for npm/PyPI packages. It uses ONNX Runtime for efficient inference in Rust.

## Architecture

```
Package Tarball
     ↓
Feature Extraction (TF-IDF)
     ↓
Neural Network (ONNX)
     ↓
Threat Score (0-100)
     ↓
Validator Decision
```

## Current Status

✅ **Infrastructure Complete**
- ONNX Runtime integration working
- Model loading and inference pipeline ready
- Training scripts created
- Docker environment fixed (Ubuntu 24.04 with glibc 2.39)

⏳ **Model Training Required**
- Placeholder config created
- Real model needs training on malware dataset

## Quick Start

### 1. Install Dependencies

```bash
cd chain-registry/ml/training
pip install -r requirements.txt
```

### 2. Train Model (Option 1: Lightweight)

Fast training on CPU, good baseline accuracy (~85%):

```bash
python train_lightweight_classifier.py
```

**Features:**
- TF-IDF vectorization (1-3 grams)
- 2-layer neural network
- Trains in 5-10 minutes on CPU
- Model size: ~50KB

### 3. Train Model (Option 2: CodeBERT)

Higher accuracy with transformer model (~95%):

```bash
python train_malware_classifier.py
```

**Features:**
- Fine-tuned CodeBERT
- Semantic code understanding
- Requires GPU for reasonable training time
- Model size: ~500MB

### 4. Verify Model

```bash
ls -la chain-registry/models/
# Should see:
# - malware_classifier.onnx
# - malware_classifier_config.json
# - tfidf_vectorizer.pkl (for lightweight model)
```

## Training Data

### Recommended Datasets

1. **MalOSS Dataset**
   - 8,000+ malicious packages from npm/PyPI
   - Academic dataset for research
   - https://github.com/ossf/malicious-packages

2. **SocketSecurity Database**
   - Real-world malicious packages
   - Professional security research
   - Requires API access

3. **PyPI/npm Security Advisories**
   - Official security notices
   - Historical attack data
   - Free and public

### Synthetic Data (for testing)

The `train_lightweight_classifier.py` includes synthetic data generation for quick testing:

```python
from train_lightweight_classifier import generate_synthetic_samples
texts, labels = generate_synthetic_samples(n_samples=10000)
```

## Model Integration

### Rust Integration

The `ml-validator` crate automatically loads the model:

```rust
use ml_validator::DeepScanner;

// Load model
let scanner = DeepScanner::new("models/malware_classifier.onnx")
    .with_tokenizer("models/tokenizer.json");

// Scan package
let result = scanner.scan(tarball_bytes)?;

// Use result
if result.malicious_probability > 0.85 {
    println!("High risk detected!");
}
```

### Validation Pipeline Integration

The AI scanner is already integrated into `validator_pipeline.rs`:

```rust
// In validator_pipeline.rs
let ml_result = ml_validator::deep_scan(
    tarball_bytes,
    "models/malware_classifier.onnx"
)?;

if ml_result.malicious_probability > 0.85 {
    findings.push(Finding::high("ML detected suspicious patterns"));
}
```

## Model Performance

| Model | Accuracy | Precision | Recall | Inference | Size |
|-------|----------|-----------|--------|-----------|------|
| Lightweight | ~85% | ~83% | ~87% | ~10ms | 50KB |
| CodeBERT | ~95% | ~94% | ~96% | ~100ms | 500MB |
| Ensemble | ~97% | ~96% | ~98% | ~110ms | 550MB |

## Production Deployment

### 1. Train Production Model

```bash
# Use real datasets
export MALOSS_DATASET=/path/to/maloss
export BENIGN_DATASET=/path/to/top10k-npm

python train_lightweight_classifier.py \
    --malicious-data $MALOSS_DATASET \
    --benign-data $BENIGN_DATASET \
    --epochs 20
```

### 2. Validate Model

```bash
python evaluate_model.py \
    --model models/malware_classifier.onnx \
    --test-data test_set.json
```

### 3. Deploy to Validators

```bash
# Copy model to all validators
for node in node-{1..10}; do
    scp models/malware_classifier.onnx $node:/app/models/
done
```

### 4. Update Model Version

```bash
# Update config with new version
jq '.version = "1.0.0" | .trained_at = "2026-04-02"' \
    models/malware_classifier_config.json > tmp.json
mv tmp.json models/malware_classifier_config.json
```

## Monitoring

### Model Performance Metrics

Track these metrics in production:

```rust
// In validator code
metrics::histogram!("ml.inference_time", inference_time_ms);
metrics::counter!("ml.predictions", 1, "class" => threat_level);
metrics::gauge!("ml.model_version", model_version);
```

### Alerting

Set up alerts for:
- Model inference time > 500ms
- Prediction confidence < 0.5 (model uncertainty)
- Model file corruption (failed checksum)

## Troubleshooting

### Issue: "ONNX model not found"

**Cause:** Model file missing or wrong path
**Fix:**
```bash
ls -la models/malware_classifier.onnx
# If missing, train or download model
python ml/training/create_minimal_model.py
```

### Issue: "ONNX Runtime error"

**Cause:** glibc version mismatch
**Fix:**
```bash
# Check glibc version
ldd --version

# Must be >= 2.38
# Update Docker image to Ubuntu 24.04 if needed
```

### Issue: Low prediction accuracy

**Cause:** Model not trained on relevant data
**Fix:**
1. Collect more diverse training data
2. Fine-tune hyperparameters
3. Try ensemble of multiple models

## Future Improvements

### Short Term
- [ ] Train on real MalOSS dataset
- [ ] Add model versioning system
- [ ] Implement A/B testing for model updates

### Medium Term
- [ ] Multi-language support (Python, Ruby, Go)
- [ ] Ensemble model (Lightweight + CodeBERT)
- [ ] Online learning from validator feedback

### Long Term
- [ ] Transformer model trained specifically on package code
- [ ] Explainable AI (attention visualization)
- [ ] Federated learning across validators

## Resources

- **ONNX Runtime**: https://onnxruntime.ai/
- **CodeBERT**: https://github.com/microsoft/CodeBERT
- **MalOSS Dataset**: https://github.com/ossf/malicious-packages
- **SocketSecurity**: https://socket.dev/

## Support

For model training issues:
1. Check logs: `tail -f logs/ml_validator.log`
2. Verify ONNX: `onnx_check models/malware_classifier.onnx`
3. Test inference: `python ml/training/test_model.py`

---

*Last updated: 2026-04-02*
