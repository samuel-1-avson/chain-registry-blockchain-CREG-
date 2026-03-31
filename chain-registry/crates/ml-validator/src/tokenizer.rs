//! Tokenization for CodeBERT Model
//!
//! This module provides tokenization for code inputs to be fed into the ML model.

use tokenizers::models::bpe::BPE;
use tokenizers::pre_tokenizers::byte_level::ByteLevel;
use tokenizers::{Result, Tokenizer};

/// Code tokenizer for ML model input
pub struct CodeTokenizer {
    tokenizer: Tokenizer,
    max_length: usize,
}

impl CodeTokenizer {
    /// Create a new code tokenizer
    pub fn new(max_length: usize) -> Result<Self> {
        // Initialize with a simple BPE tokenizer
        // In production, this would load a pre-trained tokenizer
        let mut tokenizer = Tokenizer::new(BPE::default());
        
        // Use byte-level pre-tokenization for code
        tokenizer.with_pre_tokenizer(ByteLevel::default());
        
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
}
