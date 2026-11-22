//! Resource Limits for WASM Sandbox
//!
//! This module provides resource limiting for the WASM sandbox to prevent
/// Default cap on captured WASM guest stdout and stderr (64 KiB each, VNA-16).
pub const MAX_WASM_STDIO_BYTES: usize = 64 * 1024;

/// Resource limits for sandbox execution
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum memory in bytes
    pub max_memory: usize,
    /// Maximum execution time in milliseconds
    pub max_time_ms: u64,
    /// Maximum file size in bytes
    pub max_file_size: usize,
    /// Maximum number of open files
    pub max_open_files: u32,
    /// Maximum number of syscalls per execution
    pub max_syscalls: u64,
    /// Maximum stack size in bytes
    pub max_stack_size: usize,
    /// Maximum WASM instruction fuel units
    pub max_fuel: u64,
    /// Maximum captured bytes per stdio stream (`stdout` / `stderr`)
    pub max_stdio_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory: 256 * 1024 * 1024,    // 256 MB
            max_time_ms: 30000,               // 30 seconds
            max_file_size: 100 * 1024 * 1024, // 100 MB
            max_open_files: 10,
            max_syscalls: 10000,
            max_stack_size: 8 * 1024 * 1024, // 8 MB
            max_fuel: 500_000_000,           // 500M fuel units
            max_stdio_bytes: MAX_WASM_STDIO_BYTES,
        }
    }
}

impl ResourceLimits {
    /// Create strict limits (for untrusted code)
    pub fn strict() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024,    // 64 MB
            max_time_ms: 10000,              // 10 seconds
            max_file_size: 10 * 1024 * 1024, // 10 MB
            max_open_files: 5,
            max_syscalls: 1000,
            max_stack_size: 2 * 1024 * 1024, // 2 MB
            max_fuel: 100_000_000,           // 100M fuel units
            max_stdio_bytes: MAX_WASM_STDIO_BYTES,
        }
    }

    /// Create relaxed limits (for trusted validators)
    pub fn relaxed() -> Self {
        Self {
            max_memory: 512 * 1024 * 1024,    // 512 MB
            max_time_ms: 60000,               // 60 seconds
            max_file_size: 500 * 1024 * 1024, // 500 MB
            max_open_files: 50,
            max_syscalls: 100000,
            max_stack_size: 16 * 1024 * 1024, // 16 MB
            max_fuel: 2_000_000_000,          // 2B fuel units
            max_stdio_bytes: MAX_WASM_STDIO_BYTES,
        }
    }

    /// Set memory limit
    pub fn with_memory(mut self, bytes: usize) -> Self {
        self.max_memory = bytes;
        self
    }

    /// Set time limit
    pub fn with_time_ms(mut self, ms: u64) -> Self {
        self.max_time_ms = ms;
        self
    }

    /// Set fuel limit
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.max_fuel = fuel;
        self
    }

    /// Set maximum captured bytes per stdio stream
    pub fn with_max_stdio_bytes(mut self, bytes: usize) -> Self {
        self.max_stdio_bytes = bytes;
        self
    }
}
