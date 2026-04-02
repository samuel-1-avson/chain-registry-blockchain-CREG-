"""
Export a fine-tuned CodeBERT model to ONNX for Rust inference.

Supports both:
- transformers.onnx export (preferred)
- torch.onnx.export fallback

Outputs:
- malware_classifier.onnx
- tokenizer.json (via tokenizers CLI or HuggingFace save)
"""

import os
import argparse
import torch
from transformers import AutoTokenizer, AutoModelForSequenceClassification


def export_with_transformers_onnx(model, tokenizer, output_path: str, opset: int = 14):
    """Try exporting via transformers.onnx (most reliable for HF models)."""
    from transformers.onnx import export as hf_export
    from transformers.onnx.features import FeaturesManager

    model_kind, model_onnx_config = FeaturesManager.check_supported_model_or_raise(model)
    onnx_config = model_onnx_config(model.config)

    hf_export(
        preprocessor=tokenizer,
        model=model,
        config=onnx_config,
        opset=opset,
        output=output_path,
    )
    print(f"ONNX model exported via transformers.onnx to {output_path}")


def export_with_torch_onnx(model, tokenizer, output_path: str, opset: int = 14, max_length: int = 512):
    """Fallback torch.onnx.export."""
    dummy_text = "function test() {}"
    dummy_input = tokenizer(dummy_text, return_tensors="pt", max_length=max_length, truncation=True, padding="max_length")

    torch.onnx.export(
        model,
        (dummy_input["input_ids"], dummy_input["attention_mask"]),
        output_path,
        input_names=["input_ids", "attention_mask"],
        output_names=["logits"],
        dynamic_axes={
            "input_ids": {0: "batch_size", 1: "sequence"},
            "attention_mask": {0: "batch_size", 1: "sequence"},
            "logits": {0: "batch_size"},
        },
        opset_version=opset,
        do_constant_folding=True,
    )
    print(f"ONNX model exported via torch.onnx to {output_path}")


def export_tokenizer_json(tokenizer, output_dir: str):
    """Export HuggingFace tokenizer to a single tokenizer.json if possible."""
    try:
        # Fast tokenizers expose .backend_tokenizer
        tokenizer.backend_tokenizer.save(os.path.join(output_dir, "tokenizer.json"))
        print(f"tokenizer.json saved to {output_dir}")
    except AttributeError:
        print("Tokenizer does not expose backend_tokenizer; saving via save_pretrained instead.")
        tokenizer.save_pretrained(output_dir)


def main():
    parser = argparse.ArgumentParser(description="Export trained CodeBERT to ONNX")
    parser.add_argument("--model-dir", type=str, required=True, help="Directory containing trained model")
    parser.add_argument("--output-dir", type=str, default=None, help="ONNX output directory (defaults to model-dir)")
    parser.add_argument("--opset", type=int, default=14, help="ONNX opset version")
    parser.add_argument("--max-length", type=int, default=512, help="Max sequence length")
    args = parser.parse_args()

    output_dir = args.output_dir or args.model_dir
    os.makedirs(output_dir, exist_ok=True)

    onnx_output = os.path.join(output_dir, "malware_classifier.onnx")

    print(f"Loading model from {args.model_dir}...")
    tokenizer = AutoTokenizer.from_pretrained(args.model_dir)
    model = AutoModelForSequenceClassification.from_pretrained(args.model_dir)
    model.eval()

    print("Exporting tokenizer...")
    export_tokenizer_json(tokenizer, output_dir)

    print("Exporting ONNX model...")
    try:
        export_with_transformers_onnx(model, tokenizer, onnx_output, opset=args.opset)
    except Exception as e:
        print(f"transformers.onnx export failed: {e}")
        print("Falling back to torch.onnx.export...")
        export_with_torch_onnx(model, tokenizer, onnx_output, opset=args.opset, max_length=args.max_length)

    print(f"Export complete. Artifacts in {output_dir}")


if __name__ == "__main__":
    main()
