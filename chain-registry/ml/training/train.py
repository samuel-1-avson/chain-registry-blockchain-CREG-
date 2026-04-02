"""
Training script for the CodeBERT-based malware classifier.

Fine-tunes `microsoft/codebert-base` for binary classification
(benign vs malicious) using the dataset pipeline.
"""

import os
import sys
import json
import argparse

import torch
import numpy as np
from transformers import (
    AutoTokenizer,
    AutoModelForSequenceClassification,
    TrainingArguments,
    Trainer,
    DataCollatorWithPadding,
)
from sklearn.metrics import accuracy_score, precision_recall_fscore_support

from dataset import build_dataset, tokenize_dataset


BASE_MODEL = "microsoft/codebert-base"
DEFAULT_OUTPUT_DIR = "../../chain-registry/models"


def compute_metrics(eval_pred):
    logits, labels = eval_pred
    predictions = np.argmax(logits, axis=-1)
    precision, recall, f1, _ = precision_recall_fscore_support(
        labels, predictions, average="binary"
    )
    acc = accuracy_score(labels, predictions)
    return {"accuracy": acc, "f1": f1, "precision": precision, "recall": recall}


def main():
    parser = argparse.ArgumentParser(description="Train CodeBERT malware classifier")
    parser.add_argument("--maloss-dir", type=str, default=None, help="Path to MalOSS dataset")
    parser.add_argument("--socket-file", type=str, default=None, help="Path to SocketSecurity dataset")
    parser.add_argument("--npm-file", type=str, default=None, help="Path to npm advisories JSON")
    parser.add_argument("--pypi-file", type=str, default=None, help="Path to PyPI disclosures JSON/CSV")
    parser.add_argument("--max-benign", type=int, default=10_000, help="Max benign samples")
    parser.add_argument("--epochs", type=int, default=3, help="Number of training epochs")
    parser.add_argument("--batch-size", type=int, default=8, help="Per-device batch size")
    parser.add_argument("--lr", type=float, default=2e-5, help="Learning rate")
    parser.add_argument("--max-length", type=int, default=512, help="Max token sequence length")
    parser.add_argument("--output-dir", type=str, default=DEFAULT_OUTPUT_DIR, help="Model output directory")
    parser.add_argument("--seed", type=int, default=42, help="Random seed")
    args = parser.parse_args()

    device = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"Using device: {device}")

    os.makedirs(args.output_dir, exist_ok=True)

    print("Building dataset...")
    raw_datasets = build_dataset(
        maloss_dir=args.maloss_dir,
        socket_file=args.socket_file,
        npm_file=args.npm_file,
        pypi_file=args.pypi_file,
        max_benign=args.max_benign,
        seed=args.seed,
    )
    print(f"Train size: {len(raw_datasets['train'])}, Validation size: {len(raw_datasets['validation'])}")

    tokenizer = AutoTokenizer.from_pretrained(BASE_MODEL)
    tokenized_datasets = tokenize_dataset(raw_datasets, tokenizer, max_length=args.max_length)

    model = AutoModelForSequenceClassification.from_pretrained(
        BASE_MODEL,
        num_labels=2,
    ).to(device)

    data_collator = DataCollatorWithPadding(tokenizer=tokenizer)

    training_args = TrainingArguments(
        output_dir=os.path.join(args.output_dir, "training_logs"),
        learning_rate=args.lr,
        per_device_train_batch_size=args.batch_size,
        per_device_eval_batch_size=args.batch_size,
        num_train_epochs=args.epochs,
        weight_decay=0.01,
        evaluation_strategy="epoch",
        save_strategy="epoch",
        load_best_model_at_end=True,
        logging_steps=50,
        report_to="none",
        seed=args.seed,
    )

    trainer = Trainer(
        model=model,
        args=training_args,
        train_dataset=tokenized_datasets["train"],
        eval_dataset=tokenized_datasets["validation"],
        tokenizer=tokenizer,
        data_collator=data_collator,
        compute_metrics=compute_metrics,
    )

    print("Starting fine-tuning...")
    trainer.train()

    print("Evaluating...")
    metrics = trainer.evaluate()
    print(metrics)

    print(f"Saving model to {args.output_dir}...")
    trainer.save_model(args.output_dir)
    tokenizer.save_pretrained(args.output_dir)

    config = {
        "base_model": BASE_MODEL,
        "max_length": args.max_length,
        "num_epochs": args.epochs,
        "learning_rate": args.lr,
        "batch_size": args.batch_size,
        "eval_metrics": {k: float(v) for k, v in metrics.items() if isinstance(v, (int, float, np.floating))},
        "model_dir": args.output_dir,
    }
    with open(os.path.join(args.output_dir, "malware_classifier_config.json"), "w") as f:
        json.dump(config, f, indent=2)

    print("Training complete.")


if __name__ == "__main__":
    main()
