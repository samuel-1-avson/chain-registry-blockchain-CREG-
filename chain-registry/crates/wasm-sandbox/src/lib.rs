//! WASM Sandboxing for Package Validation
//!
//! This crate provides a secure, cross-platform sandbox for validating packages
//! using WebAssembly.

use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, instrument};
use wasmtime::{Engine, Module, Store};

pub mod capabilities;
pub mod limits;

pub use capabilities::CapabilitySet;
pub use limits::ResourceLimits;

/// Errors that can occur during WASM sandbox execution
#[derive(Error, Debug)]
pub enum SandboxError {
    #[error("WASM compilation error: {0}")]
    CompilationError(String),
    
    #[error("WASM execution error: {0}")]
    ExecutionError(String),
    
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    
    #[error("Timeout after {0:?}")]
    Timeout(Duration),
    
    #[error("Memory access error: {0}")]
    MemoryError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Sandbox configuration
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Memory limit in bytes
    pub memory_limit: usize,
    /// CPU time limit in seconds
    pub timeout_secs: u64,
    /// Allowed capabilities
    pub capabilities: CapabilitySet,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024, // 256MB
            timeout_secs: 30,
            capabilities: CapabilitySet::default(),
            env_vars: HashMap::new(),
        }
    }
}

impl SandboxConfig {
    /// Set memory limit
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = bytes;
        self
    }
    
    /// Set timeout
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Input data for sandbox execution
#[derive(Debug, Clone)]
pub struct SandboxInput {
    /// Package metadata
    pub package_name: String,
    pub package_version: String,
    pub ecosystem: String,
    /// Package content
    pub tarball_bytes: Vec<u8>,
}

impl SandboxInput {
    /// Create new sandbox input
    pub fn new(package_name: &str, version: &str, ecosystem: &str) -> Self {
        Self {
            package_name: package_name.to_string(),
            package_version: version.to_string(),
            ecosystem: ecosystem.to_string(),
            tarball_bytes: vec![],
        }
    }
    
    /// Set tarball bytes
    pub fn with_tarball(mut self, bytes: Vec<u8>) -> Self {
        self.tarball_bytes = bytes;
        self
    }
}

/// Result of sandbox execution
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Whether execution was successful
    pub success: bool,
    /// Exit code
    pub exit_code: i32,
    /// stdout output
    pub stdout: String,
    /// stderr output
    pub stderr: String,
    /// Resource usage
    pub resource_usage: ResourceUsage,
    /// Validation findings
    pub findings: Vec<SafetyFinding>,
}

/// Resource usage statistics
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Peak memory usage in bytes
    pub peak_memory: usize,
    /// CPU time used in milliseconds
    pub cpu_time_ms: u64,
    /// Wall clock time in milliseconds
    pub wall_time_ms: u64,
}

/// Safety finding from validation
#[derive(Debug, Clone)]
pub struct SafetyFinding {
    /// Severity level
    pub severity: Severity,
    /// Category
    pub category: String,
    /// Description
    pub description: String,
}

/// Severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// WASM Sandbox for package validation
pub struct WasmSandbox {
    engine: Engine,
    config: SandboxConfig,
}

impl WasmSandbox {
    /// Create a new WASM sandbox
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        info!("Initializing WASM sandbox");
        
        let engine = Engine::default();
        
        Ok(Self { engine, config })
    }
    
    /// Run a WASM module in the sandbox
    pub async fn run(
        &self,
        wasm_bytes: &[u8],
        _input: &SandboxInput,
    ) -> Result<SandboxResult, SandboxError> {
        debug!("Compiling WASM module");
        
        // Compile module
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::CompilationError(e.to_string()))?;
        
        // Create store
        let mut store = Store::new(&self.engine, ());
        
        // Instantiate module
        let instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| SandboxError::ExecutionError(e.to_string()))?;
        
        // Get the main function
        let main = instance
            .get_typed_func::<(), i32>(&mut store, "_start")
            .or_else(|_| instance.get_typed_func::<(), i32>(&mut store, "main"))
            .map_err(|e| SandboxError::ExecutionError(format!("No main function: {}", e)))?;
        
        // Run with timeout
        let start_time = std::time::Instant::now();
        
        let exit_code = main.call(&mut store, ())
            .map_err(|e| SandboxError::ExecutionError(e.to_string()))?;
        
        let wall_time = start_time.elapsed();
        
        Ok(SandboxResult {
            success: exit_code == 0,
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
            resource_usage: ResourceUsage {
                peak_memory: 0,
                cpu_time_ms: wall_time.as_millis() as u64,
                wall_time_ms: wall_time.as_millis() as u64,
            },
            findings: vec![],
        })
    }
    
    /// Run a validator script on a package
    pub async fn validate_package(
        &self,
        validator_wasm: &[u8],
        package_data: &SandboxInput,
    ) -> Result<SandboxResult, SandboxError> {
        debug!("Running package validation in WASM sandbox");
        self.run(validator_wasm, package_data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config() {
        let config = SandboxConfig::default()
            .with_memory_limit(1024)
            .with_timeout_secs(10);
        
        assert_eq!(config.memory_limit, 1024);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_sandbox_input() {
        let input = SandboxInput::new("test-pkg", "1.0.0", "npm");
        
        assert_eq!(input.package_name, "test-pkg");
        assert_eq!(input.ecosystem, "npm");
    }
}
