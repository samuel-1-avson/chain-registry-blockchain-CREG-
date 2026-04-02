#!/usr/bin/env python3
"""
Lightweight Malware Classifier Training Pipeline

This script trains a fast, lightweight classifier for malicious package detection
using TF-IDF features and a neural network. It's designed to:
1. Train quickly on CPU (no GPU required)
2. Export to ONNX for Rust inference
3. Provide reasonable baseline performance (~85-90% accuracy)
4. Be replaced later with a full CodeBERT model

In production, use `train_malware_classifier.py` for the full CodeBERT model.
"""

import os
import sys
import json
import pickle
import numpy as np
from typing import List, Tuple, Dict
import torch
import torch.nn as nn
from torch.utils.data import Dataset, DataLoader
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.model_selection import train_test_split
from sklearn.metrics import accuracy_score, precision_recall_fscore_support, classification_report
import onnx
import onnxruntime as ort

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
OUTPUT_DIR = "chain-registry/models"
VOCAB_SIZE = 10000  # Max features for TF-IDF
MAX_LENGTH = 512    # Max tokens per sample
HIDDEN_DIM = 256
BATCH_SIZE = 32
LEARNING_RATE = 0.001
NUM_EPOCHS = 10
TEST_SIZE = 0.2
RANDOM_STATE = 42

os.makedirs(OUTPUT_DIR, exist_ok=True)

# ---------------------------------------------------------------------------
# Synthetic Dataset Generator (for demonstration)
# ---------------------------------------------------------------------------

MALICIOUS_PATTERNS = [
    # Obfuscation patterns
    "eval(atob(",
    "Function(atob(",
    "String.fromCharCode(",
    "parseInt(String.fromCharCode(",
    "unescape(encodeURIComponent(",
    # Network exfiltration
    "fetch('http://",
    "XMLHttpRequest",
    "navigator.sendBeacon",
    "WebSocket('wss://evil",
    # Crypto mining
    "CoinHive",
    "WebAssembly.instantiate",
    "cryptoNight",
    "new Worker('data:text/javascript",
    # Shell execution
    "child_process",
    "execSync(",
    "spawn(",
    "require('child_process')",
    # Credential theft
    "localStorage.getItem('password')",
    "document.cookie.match",
    "process.env.API_KEY",
    # Suspicious dynamic code
    "new Function(",
    "setTimeout(String.fromCharCode",
    "document.write(atob",
]

BENIGN_PATTERNS = [
    # Standard imports
    "import React from 'react'",
    "const express = require('express')",
    "import { useState } from 'react'",
    "const axios = require('axios')",
    # Normal functions
    "function calculateSum(a, b) { return a + b; }",
    "const handleClick = () => { setCount(count + 1); }",
    "app.get('/', (req, res) => { res.send('Hello'); })",
    # Documentation
    "/** @param {string} name - The user name */",
    "// TODO: Refactor this function",
    "/* LICENSE: MIT */",
    # Normal async
    "async function fetchData() { const res = await fetch('/api'); }",
    "Promise.all([",
    "setInterval(() => update(), 5000)",
    # Standard DOM
    "document.getElementById('app')",
    "element.addEventListener('click', handler)",
    "window.location.href = '/home'",
]


def generate_synthetic_samples(n_samples: int = 5000) -> Tuple[List[str], List[int]]:
    """Generate synthetic training data."""
    print(f"Generating {n_samples} synthetic samples...")
    
    texts = []
    labels = []
    
    # Generate malicious samples
    for i in range(n_samples // 2):
        # Combine multiple malicious patterns with noise
        n_patterns = np.random.randint(1, 4)
        patterns = np.random.choice(MALICIOUS_PATTERNS, n_patterns, replace=False)
        
        # Add some benign code as noise
        n_benign = np.random.randint(2, 5)
        benign = np.random.choice(BENIGN_PATTERNS, n_benign, replace=False)
        
        # Mix together
        code_lines = list(patterns) + list(benign)
        np.random.shuffle(code_lines)
        
        # Add random variable names and structure
        code = "\n".join(code_lines)
        code = f"function sample_{i}() {{\n{code}\n}}"
        
        texts.append(code)
        labels.append(1)  # Malicious
    
    # Generate benign samples
    for i in range(n_samples // 2):
        n_patterns = np.random.randint(5, 15)
        patterns = np.random.choice(BENIGN_PATTERNS, n_patterns, replace=True)
        
        code = "\n".join(patterns)
        code = f"function sample_{i}() {{\n{code}\n}}"
        
        texts.append(code)
        labels.append(0)  # Benign
    
    # Shuffle
    indices = np.random.permutation(len(texts))
    return [texts[i] for i in indices], [labels[i] for i in indices]


# ---------------------------------------------------------------------------
# Neural Network Model
# ---------------------------------------------------------------------------

class MalwareClassifier(nn.Module):
    """Simple feedforward network for TF-IDF features."""
    
    def __init__(self, input_dim: int, hidden_dim: int = 256):
        super().__init__()
        self.network = nn.Sequential(
            nn.Linear(input_dim, hidden_dim),
            nn.ReLU(),
            nn.Dropout(0.3),
            nn.Linear(hidden_dim, hidden_dim // 2),
            nn.ReLU(),
            nn.Dropout(0.2),
            nn.Linear(hidden_dim // 2, 2),  # Binary classification
        )
    
    def forward(self, x):
        return self.network(x)


class TfidfDataset(Dataset):
    """PyTorch Dataset wrapper for TF-IDF vectors."""
    
    def __init__(self, X: np.ndarray, y: List[int]):
        self.X = torch.FloatTensor(X.toarray() if hasattr(X, 'toarray') else X)
        self.y = torch.LongTensor(y)
    
    def __len__(self):
        return len(self.y)
    
    def __getitem__(self, idx):
        return self.X[idx], self.y[idx]


# ---------------------------------------------------------------------------
# Training
# ---------------------------------------------------------------------------

def train():
    print("=" * 60)
    print("Lightweight Malware Classifier Training")
    print("=" * 60)
    
    # Generate synthetic data
    texts, labels = generate_synthetic_samples(n_samples=10000)
    
    # Split data
    X_train, X_test, y_train, y_test = train_test_split(
        texts, labels, test_size=TEST_SIZE, random_state=RANDOM_STATE, stratify=labels
    )
    
    print(f"\nTraining samples: {len(X_train)}")
    print(f"Test samples: {len(X_test)}")
    
    # Create TF-IDF vectorizer
    print("\nFitting TF-IDF vectorizer...")
    vectorizer = TfidfVectorizer(
        max_features=VOCAB_SIZE,
        ngram_range=(1, 3),  # Unigrams, bigrams, trigrams
        min_df=2,
        max_df=0.95,
    )
    X_train_tfidf = vectorizer.fit_transform(X_train)
    X_test_tfidf = vectorizer.transform(X_test)
    
    print(f"TF-IDF feature shape: {X_train_tfidf.shape}")
    
    # Save vectorizer
    vectorizer_path = os.path.join(OUTPUT_DIR, "tfidf_vectorizer.pkl")
    with open(vectorizer_path, 'wb') as f:
        pickle.dump(vectorizer, f)
    print(f"Vectorizer saved to {vectorizer_path}")
    
    # Create datasets
    train_dataset = TfidfDataset(X_train_tfidf, y_train)
    test_dataset = TfidfDataset(X_test_tfidf, y_test)
    
    train_loader = DataLoader(train_dataset, batch_size=BATCH_SIZE, shuffle=True)
    test_loader = DataLoader(test_dataset, batch_size=BATCH_SIZE)
    
    # Create model
    input_dim = X_train_tfidf.shape[1]
    model = MalwareClassifier(input_dim, HIDDEN_DIM)
    
    device = torch.device('cuda' if torch.cuda.is_available() else 'cpu')
    print(f"\nUsing device: {device}")
    model = model.to(device)
    
    # Training setup
    criterion = nn.CrossEntropyLoss()
    optimizer = torch.optim.Adam(model.parameters(), lr=LEARNING_RATE)
    
    # Training loop
    print("\nTraining model...")
    model.train()
    for epoch in range(NUM_EPOCHS):
        total_loss = 0
        correct = 0
        total = 0
        
        for batch_X, batch_y in train_loader:
            batch_X, batch_y = batch_X.to(device), batch_y.to(device)
            
            optimizer.zero_grad()
            outputs = model(batch_X)
            loss = criterion(outputs, batch_y)
            loss.backward()
            optimizer.step()
            
            total_loss += loss.item()
            _, predicted = torch.max(outputs.data, 1)
            total += batch_y.size(0)
            correct += (predicted == batch_y).sum().item()
        
        accuracy = 100 * correct / total
        avg_loss = total_loss / len(train_loader)
        print(f"Epoch [{epoch+1}/{NUM_EPOCHS}] Loss: {avg_loss:.4f} Accuracy: {accuracy:.2f}%")
    
    # Evaluation
    print("\nEvaluating model...")
    model.eval()
    all_preds = []
    all_labels = []
    
    with torch.no_grad():
        for batch_X, batch_y in test_loader:
            batch_X = batch_X.to(device)
            outputs = model(batch_X)
            _, predicted = torch.max(outputs, 1)
            all_preds.extend(predicted.cpu().numpy())
            all_labels.extend(batch_y.numpy())
    
    # Metrics
    accuracy = accuracy_score(all_labels, all_preds)
    precision, recall, f1, _ = precision_recall_fscore_support(all_labels, all_preds, average='binary')
    
    print(f"\nTest Results:")
    print(f"  Accuracy:  {accuracy:.4f}")
    print(f"  Precision: {precision:.4f}")
    print(f"  Recall:    {recall:.4f}")
    print(f"  F1 Score:  {f1:.4f}")
    
    print("\nDetailed Classification Report:")
    print(classification_report(all_labels, all_preds, target_names=['Benign', 'Malicious']))
    
    # -----------------------------------------------------------------------
    # Export to ONNX
    # -----------------------------------------------------------------------
    print("\nExporting to ONNX...")
    
    model_path = os.path.join(OUTPUT_DIR, "malware_classifier.onnx")
    
    # Create dummy input
    dummy_input = torch.randn(1, input_dim)
    
    # Export
    torch.onnx.export(
        model.cpu(),
        dummy_input,
        model_path,
        input_names=["input"],
        output_names=["logits"],
        dynamic_axes={
            "input": {0: "batch_size"},
            "logits": {0: "batch_size"},
        },
        opset_version=14,
        do_constant_folding=True,
    )
    
    print(f"ONNX model saved to {model_path}")
    
    # Verify ONNX model
    onnx_model = onnx.load(model_path)
    onnx.checker.check_model(onnx_model)
    print("ONNX model verification passed!")
    
    # Test ONNX inference
    ort_session = ort.InferenceSession(model_path)
    test_input = np.random.randn(1, input_dim).astype(np.float32)
    ort_outputs = ort_session.run(None, {"input": test_input})
    print(f"ONNX inference test passed! Output shape: {ort_outputs[0].shape}")
    
    # Save config
    config = {
        "model_type": "tfidf_neural_net",
        "input_dim": int(input_dim),
        "hidden_dim": HIDDEN_DIM,
        "vocab_size": VOCAB_SIZE,
        "accuracy": float(accuracy),
        "precision": float(precision),
        "recall": float(recall),
        "f1_score": float(f1),
        "num_epochs": NUM_EPOCHS,
        "learning_rate": LEARNING_RATE,
    }
    
    config_path = os.path.join(OUTPUT_DIR, "malware_classifier_config.json")
    with open(config_path, 'w') as f:
        json.dump(config, f, indent=2)
    print(f"Config saved to {config_path}")
    
    # Save PyTorch model (for retraining)
    pytorch_path = os.path.join(OUTPUT_DIR, "malware_classifier.pt")
    torch.save(model.state_dict(), pytorch_path)
    print(f"PyTorch weights saved to {pytorch_path}")
    
    print("\n" + "=" * 60)
    print("Training Complete!")
    print("=" * 60)
    print(f"\nArtifacts saved to {OUTPUT_DIR}:")
    print(f"  - malware_classifier.onnx (ONNX model)")
    print(f"  - malware_classifier.pt (PyTorch weights)")
    print(f"  - tfidf_vectorizer.pkl (TF-IDF vectorizer)")
    print(f"  - malware_classifier_config.json (Metadata)")
    
    return model, vectorizer, config


if __name__ == "__main__":
    train()
