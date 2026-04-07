//! Tokenization for CodeBERT Model
//!
//! This module provides tokenization for code inputs to be fed into the ML model.

use tokenizers::models::bpe::BPE;
use tokenizers::{Result, Tokenizer};

/// Code tokenizer for ML model input
pub struct CodeTokenizer {
    tokenizer: Tokenizer,
    max_length: usize,
}

impl CodeTokenizer {
    /// Create a new code tokenizer
    pub fn new(max_length: usize) -> Result<Self> {
        // Build a vocabulary of byte-fallback tokens.
        // Maps <0x00>..<0xFF> → token IDs 1..=256, keeping 0 for padding.
        let mut vocab = std::collections::HashMap::new();
        for b in 0u8..=255u8 {
            vocab.insert(format!("<0x{:02X}>", b), b as u32 + 1);
        }

        let bpe = BPE::builder()
            .vocab_and_merges(vocab, vec![])
            .byte_fallback(true)
            .build()?;
        let tokenizer = Tokenizer::new(bpe);

        Ok(Self {
            tokenizer,
            max_length,
        })
    }

    /// Load a tokenizer from a file
    pub fn from_file<P: AsRef<std::path::Path>>(path: P, max_length: usize) -> Result<Self> {
        let tokenizer = Tokenizer::from_file(path)?;

        Ok(Self {
            tokenizer,
            max_length,
        })
    }

    /// Tokenize code into input IDs
    pub fn encode(&self, code: &str) -> Result<Vec<u32>> {
        let encoding = self.tokenizer.encode(code, true)?;
        let mut ids = encoding.get_ids().to_vec();

        // Truncate or pad to max_length
        if ids.len() > self.max_length {
            ids.truncate(self.max_length);
        } else {
            // Pad with zeros (assuming 0 is padding token)
            while ids.len() < self.max_length {
                ids.push(0);
            }
        }

        Ok(ids)
    }

    /// Tokenize code into input IDs and attention mask
    pub fn encode_with_attention(&self, code: &str) -> Result<(Vec<u32>, Vec<u32>)> {
        let encoding = self.tokenizer.encode(code, true)?;
        let mut ids = encoding.get_ids().to_vec();
        let mut mask = encoding.get_attention_mask().to_vec();

        // Truncate or pad to max_length
        if ids.len() > self.max_length {
            ids.truncate(self.max_length);
            mask.truncate(self.max_length);
        } else {
            while ids.len() < self.max_length {
                ids.push(0);
                mask.push(0);
            }
        }

        Ok((ids, mask))
    }

    /// Decode token IDs back to string
    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        self.tokenizer.decode(ids, true)
    }

    /// Get vocabulary size
    pub fn vocab_size(&self) -> usize {
        self.tokenizer.get_vocab_size(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_code() {
        let tokenizer = CodeTokenizer::new(512).unwrap();
        let code = "function test() { return 42; }";

        let ids = tokenizer.encode(code).unwrap();
        assert_eq!(ids.len(), 512);

        // Check that we got non-padding tokens
        let non_padding = ids.iter().filter(|&&id| id != 0).count();
        assert!(non_padding > 0);
    }

    #[test]
    fn test_tokenize_with_attention() {
        let tokenizer = CodeTokenizer::new(512).unwrap();
        let code = "function test() { return 42; }";

        let (ids, mask) = tokenizer.encode_with_attention(code).unwrap();
        assert_eq!(ids.len(), 512);
        assert_eq!(mask.len(), 512);

        let non_padding = mask.iter().filter(|&&m| m == 1).count();
        assert!(non_padding > 0);
    }
}
