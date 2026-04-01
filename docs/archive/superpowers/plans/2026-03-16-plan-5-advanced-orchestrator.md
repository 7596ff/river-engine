# Advanced Orchestrator Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the minimal orchestrator with model discovery, GPU/VRAM tracking, llama-server process management, and LiteLLM integration.

**Architecture:** New modules (discovery, resources, process) extend the existing orchestrator. GGUF files are scanned at startup, resources tracked in-memory, llama-server processes spawned on-demand with health monitoring and idle eviction.

**Tech Stack:** Rust, tokio, axum, serde. New: `which` crate for PATH lookup, process spawning via `tokio::process`.

**Spec:** `docs/superpowers/specs/2026-03-16-orchestrator-advanced-design.md`

---

## Chunk 1: GGUF Parsing and Model Discovery

### Task 1: Add new dependencies to Cargo.toml

**Files:**
- Modify: `crates/river-orchestrator/Cargo.toml`

- [ ] **Step 1: Add dependencies**

Add `which` for llama-server lookup and `reqwest` for health checks:

```toml
[dependencies]
# ... existing deps ...
which = "7.0"
reqwest.workspace = true
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p river-orchestrator`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/Cargo.toml
git commit -m "feat(orchestrator): add which and reqwest dependencies"
```

---

### Task 2: Create GGUF metadata types

**Files:**
- Create: `crates/river-orchestrator/src/discovery/mod.rs`
- Create: `crates/river-orchestrator/src/discovery/gguf.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Write test for QuantizationType parsing**

Create `crates/river-orchestrator/src/discovery/gguf.rs`:

```rust
//! GGUF file header parsing

use serde::Serialize;

/// Quantization type parsed from GGUF filename or metadata
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum QuantizationType {
    Q4_0,
    Q4_1,
    Q4_K_M,
    Q4_K_S,
    Q5_0,
    Q5_1,
    Q5_K_M,
    Q5_K_S,
    Q6_K,
    Q8_0,
    F16,
    F32,
    Unknown(String),
}

impl QuantizationType {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "Q4_0" => Self::Q4_0,
            "Q4_1" => Self::Q4_1,
            "Q4_K_M" => Self::Q4_K_M,
            "Q4_K_S" => Self::Q4_K_S,
            "Q5_0" => Self::Q5_0,
            "Q5_1" => Self::Q5_1,
            "Q5_K_M" => Self::Q5_K_M,
            "Q5_K_S" => Self::Q5_K_S,
            "Q6_K" => Self::Q6_K,
            "Q8_0" => Self::Q8_0,
            "F16" => Self::F16,
            "F32" => Self::F32,
            _ => Self::Unknown(s.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantization_type_parsing() {
        assert_eq!(QuantizationType::from_str("q4_k_m"), QuantizationType::Q4_K_M);
        assert_eq!(QuantizationType::from_str("Q8_0"), QuantizationType::Q8_0);
        assert_eq!(QuantizationType::from_str("F16"), QuantizationType::F16);
    }

    #[test]
    fn test_quantization_type_unknown() {
        match QuantizationType::from_str("custom") {
            QuantizationType::Unknown(s) => assert_eq!(s, "custom"),
            _ => panic!("Expected Unknown variant"),
        }
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p river-orchestrator quantization_type`
Expected: 2 tests pass

- [ ] **Step 3: Add GgufMetadata struct**

Add to `crates/river-orchestrator/src/discovery/gguf.rs`:

```rust
use std::path::PathBuf;

/// Metadata extracted from a GGUF file
#[derive(Debug, Clone)]
pub struct GgufMetadata {
    pub name: String,
    pub architecture: String,
    pub parameters: u64,
    pub quantization: QuantizationType,
    pub context_length: u32,
    pub layers: u32,
    pub hidden_dim: u32,
    pub file_size: u64,
}

impl GgufMetadata {
    /// Estimate VRAM required to load this model (in bytes)
    pub fn estimate_vram(&self) -> u64 {
        let kv_cache = self.estimate_kv_cache();
        let overhead = 500 * 1024 * 1024; // 500MB for llama-server
        self.file_size + kv_cache + overhead
    }

    fn estimate_kv_cache(&self) -> u64 {
        // KV cache: 4 bytes per token per layer * hidden_dim * 2 (K+V)
        let bytes_per_token = (self.layers as u64) * (self.hidden_dim as u64) * 4;
        (self.context_length as u64) * bytes_per_token
    }
}
```

- [ ] **Step 4: Add VRAM estimation test**

Add to tests in `gguf.rs`:

```rust
#[test]
fn test_vram_estimation() {
    let metadata = GgufMetadata {
        name: "test-model".to_string(),
        architecture: "llama".to_string(),
        parameters: 7_000_000_000,
        quantization: QuantizationType::Q4_K_M,
        context_length: 8192,
        layers: 32,
        hidden_dim: 4096,
        file_size: 4_000_000_000, // 4GB
    };

    let vram = metadata.estimate_vram();
    // file_size + kv_cache + overhead
    // 4GB + (8192 * 32 * 4096 * 4) + 500MB
    // 4GB + ~4.3GB + 0.5GB = ~8.8GB
    assert!(vram > 8_000_000_000);
    assert!(vram < 10_000_000_000);
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p river-orchestrator vram_estimation`
Expected: PASS

- [ ] **Step 6: Create discovery module**

Create `crates/river-orchestrator/src/discovery/mod.rs`:

```rust
//! Model discovery from GGUF files

pub mod gguf;

pub use gguf::{GgufMetadata, QuantizationType};
```

- [ ] **Step 7: Add discovery module to lib.rs**

Modify `crates/river-orchestrator/src/lib.rs`, add after line 4:

```rust
pub mod discovery;
```

- [ ] **Step 8: Run all tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 9: Commit**

```bash
git add crates/river-orchestrator/src/discovery/
git add crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add GGUF metadata types and VRAM estimation"
```

---

### Task 3: Parse GGUF file headers

**Files:**
- Modify: `crates/river-orchestrator/src/discovery/gguf.rs`

- [ ] **Step 1: Add GGUF magic constant and header parsing**

Add to `gguf.rs` before the tests module:

```rust
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use river_core::RiverError;

/// GGUF magic number
const GGUF_MAGIC: u32 = 0x46554747; // "GGUF" in little-endian

/// Parse GGUF file and extract metadata
pub fn parse_gguf(path: &PathBuf) -> Result<GgufMetadata, RiverError> {
    let file = File::open(path)
        .map_err(|e| RiverError::orchestrator(format!("Failed to open {}: {}", path.display(), e)))?;

    let file_size = file.metadata()
        .map_err(|e| RiverError::orchestrator(format!("Failed to get file size: {}", e)))?
        .len();

    let mut reader = BufReader::new(file);

    // Read magic number
    let magic = read_u32(&mut reader)?;
    if magic != GGUF_MAGIC {
        return Err(RiverError::orchestrator(format!(
            "Invalid GGUF magic: expected {:08x}, got {:08x}",
            GGUF_MAGIC, magic
        )));
    }

    // Read version
    let version = read_u32(&mut reader)?;
    if version < 2 || version > 3 {
        return Err(RiverError::orchestrator(format!(
            "Unsupported GGUF version: {}",
            version
        )));
    }

    // Read tensor count and metadata KV count
    let _tensor_count = read_u64(&mut reader)?;
    let metadata_kv_count = read_u64(&mut reader)?;

    // Parse metadata key-value pairs
    let mut name = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut architecture = "unknown".to_string();
    let mut context_length = 4096u32;
    let mut layers = 32u32;
    let mut hidden_dim = 4096u32;

    for _ in 0..metadata_kv_count {
        let (key, value) = read_metadata_kv(&mut reader)?;
        match key.as_str() {
            "general.name" => name = value.as_string().unwrap_or(name),
            "general.architecture" => architecture = value.as_string().unwrap_or(architecture),
            k if k.ends_with(".context_length") => {
                context_length = value.as_u32().unwrap_or(context_length);
            }
            k if k.ends_with(".block_count") => {
                layers = value.as_u32().unwrap_or(layers);
            }
            k if k.ends_with(".embedding_length") => {
                hidden_dim = value.as_u32().unwrap_or(hidden_dim);
            }
            _ => {}
        }
    }

    // Extract quantization from filename (most reliable)
    let quantization = extract_quantization_from_filename(path);

    // Estimate parameters from file size and quantization
    let parameters = estimate_parameters(file_size, &quantization);

    Ok(GgufMetadata {
        name,
        architecture,
        parameters,
        quantization,
        context_length,
        layers,
        hidden_dim,
        file_size,
    })
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, RiverError> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read u32: {}", e)))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(reader: &mut R) -> Result<u64, RiverError> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read u64: {}", e)))?;
    Ok(u64::from_le_bytes(buf))
}

/// GGUF metadata value types
#[derive(Debug)]
enum GgufValue {
    U32(u32),
    I32(i32),
    F32(f32),
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
    String(String),
    Array(Vec<GgufValue>),
}

impl GgufValue {
    fn as_string(&self) -> Option<String> {
        match self {
            GgufValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn as_u32(&self) -> Option<u32> {
        match self {
            GgufValue::U32(v) => Some(*v),
            GgufValue::I32(v) => Some(*v as u32),
            GgufValue::U64(v) => Some(*v as u32),
            _ => None,
        }
    }
}

fn read_metadata_kv<R: Read>(reader: &mut R) -> Result<(String, GgufValue), RiverError> {
    // Read key length and key
    let key_len = read_u64(reader)? as usize;
    let mut key_buf = vec![0u8; key_len];
    reader.read_exact(&mut key_buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read key: {}", e)))?;
    let key = String::from_utf8_lossy(&key_buf).to_string();

    // Read value type
    let value_type = read_u32(reader)?;
    let value = read_gguf_value(reader, value_type)?;

    Ok((key, value))
}

fn read_gguf_value<R: Read>(reader: &mut R, value_type: u32) -> Result<GgufValue, RiverError> {
    match value_type {
        0 => Ok(GgufValue::U32(read_u32(reader)?)),          // UINT32
        1 => Ok(GgufValue::I32(read_u32(reader)? as i32)),   // INT32
        2 => Ok(GgufValue::F32(f32::from_le_bytes({         // FLOAT32
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).map_err(|e| RiverError::orchestrator(e.to_string()))?;
            buf
        }))),
        3 => Ok(GgufValue::Bool(read_u32(reader)? != 0)),    // BOOL (stored as u32 in v2)
        4 => {                                               // STRING
            let len = read_u64(reader)? as usize;
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)
                .map_err(|e| RiverError::orchestrator(format!("Failed to read string: {}", e)))?;
            Ok(GgufValue::String(String::from_utf8_lossy(&buf).to_string()))
        }
        5 => {                                               // ARRAY
            let elem_type = read_u32(reader)?;
            let len = read_u64(reader)? as usize;
            let mut arr = Vec::with_capacity(len);
            for _ in 0..len {
                arr.push(read_gguf_value(reader, elem_type)?);
            }
            Ok(GgufValue::Array(arr))
        }
        6 => Ok(GgufValue::U64(read_u64(reader)?)),          // UINT64
        7 => Ok(GgufValue::I64(read_u64(reader)? as i64)),   // INT64
        8 => Ok(GgufValue::F64(f64::from_le_bytes({          // FLOAT64
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).map_err(|e| RiverError::orchestrator(e.to_string()))?;
            buf
        }))),
        10 => Ok(GgufValue::Bool({                           // BOOL (v3, stored as u8)
            let mut buf = [0u8; 1];
            reader.read_exact(&mut buf).map_err(|e| RiverError::orchestrator(e.to_string()))?;
            buf[0] != 0
        })),
        _ => Err(RiverError::orchestrator(format!("Unknown GGUF value type: {}", value_type))),
    }
}

fn extract_quantization_from_filename(path: &PathBuf) -> QuantizationType {
    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_uppercase();

    // Common quantization patterns in filenames
    let patterns = [
        "Q4_K_M", "Q4_K_S", "Q4_0", "Q4_1",
        "Q5_K_M", "Q5_K_S", "Q5_0", "Q5_1",
        "Q6_K", "Q8_0", "F16", "F32",
    ];

    for pattern in patterns {
        if filename.contains(pattern) {
            return QuantizationType::from_str(pattern);
        }
    }

    QuantizationType::Unknown("unknown".to_string())
}

fn estimate_parameters(file_size: u64, quantization: &QuantizationType) -> u64 {
    // Rough estimate: divide file size by bytes per parameter
    let bytes_per_param = match quantization {
        QuantizationType::Q4_0 | QuantizationType::Q4_1 |
        QuantizationType::Q4_K_M | QuantizationType::Q4_K_S => 0.5,
        QuantizationType::Q5_0 | QuantizationType::Q5_1 |
        QuantizationType::Q5_K_M | QuantizationType::Q5_K_S => 0.625,
        QuantizationType::Q6_K => 0.75,
        QuantizationType::Q8_0 => 1.0,
        QuantizationType::F16 => 2.0,
        QuantizationType::F32 => 4.0,
        QuantizationType::Unknown(_) => 0.5, // Assume Q4 as default
    };
    (file_size as f64 / bytes_per_param) as u64
}
```

- [ ] **Step 2: Add test for quantization extraction**

Add to tests:

```rust
#[test]
fn test_extract_quantization_from_filename() {
    use std::path::PathBuf;

    let path = PathBuf::from("/models/llama-3-8b-q4_k_m.gguf");
    assert_eq!(extract_quantization_from_filename(&path), QuantizationType::Q4_K_M);

    let path = PathBuf::from("/models/qwen2-7b-Q8_0.gguf");
    assert_eq!(extract_quantization_from_filename(&path), QuantizationType::Q8_0);
}

#[test]
fn test_estimate_parameters() {
    // 4GB Q4_K_M file should be ~8B params
    let params = estimate_parameters(4_000_000_000, &QuantizationType::Q4_K_M);
    assert!(params > 7_000_000_000);
    assert!(params < 9_000_000_000);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator extract_quantization`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/discovery/gguf.rs
git commit -m "feat(orchestrator): add GGUF header parsing"
```

---

### Task 4: Create model scanner

**Files:**
- Create: `crates/river-orchestrator/src/discovery/scanner.rs`
- Modify: `crates/river-orchestrator/src/discovery/mod.rs`

- [ ] **Step 1: Create LocalModel struct and scanner**

Create `crates/river-orchestrator/src/discovery/scanner.rs`:

```rust
//! Model directory scanning

use super::gguf::{parse_gguf, GgufMetadata};
use river_core::RiverError;
use std::collections::HashSet;
use std::path::PathBuf;

/// A local GGUF model
#[derive(Debug, Clone)]
pub struct LocalModel {
    pub id: String,
    pub path: PathBuf,
    pub metadata: GgufMetadata,
}

/// Scanner for discovering GGUF models in directories
pub struct ModelScanner {
    model_dirs: Vec<PathBuf>,
}

impl ModelScanner {
    /// Create a new scanner with the given directories
    pub fn new(model_dirs: Vec<PathBuf>) -> Self {
        Self { model_dirs }
    }

    /// Scan all directories and return discovered models
    pub fn scan(&self) -> Vec<LocalModel> {
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut models = Vec::new();

        for dir in &self.model_dirs {
            if !dir.exists() {
                tracing::warn!("Model directory does not exist: {}", dir.display());
                continue;
            }

            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::warn!("Failed to read directory {}: {}", dir.display(), e);
                    continue;
                }
            };

            for entry in entries.flatten() {
                let path = entry.path();

                // Only process .gguf files
                if path.extension().and_then(|s| s.to_str()) != Some("gguf") {
                    continue;
                }

                // Extract model ID from filename
                let id = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(stem) => stem.to_string(),
                    None => continue,
                };

                // Check for duplicates
                if seen_ids.contains(&id) {
                    tracing::warn!(
                        "Skipping duplicate model ID '{}' at {}",
                        id, path.display()
                    );
                    continue;
                }

                // Parse GGUF metadata
                match parse_gguf(&path) {
                    Ok(metadata) => {
                        seen_ids.insert(id.clone());
                        models.push(LocalModel { id, path, metadata });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse GGUF {}: {}", path.display(), e);
                    }
                }
            }
        }

        tracing::info!("Discovered {} local models", models.len());
        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scanner_empty_dirs() {
        let scanner = ModelScanner::new(vec![]);
        let models = scanner.scan();
        assert!(models.is_empty());
    }

    #[test]
    fn test_scanner_nonexistent_dir() {
        let scanner = ModelScanner::new(vec![PathBuf::from("/nonexistent/path")]);
        let models = scanner.scan();
        assert!(models.is_empty());
    }
}
```

- [ ] **Step 2: Update discovery mod.rs**

Replace `crates/river-orchestrator/src/discovery/mod.rs`:

```rust
//! Model discovery from GGUF files

pub mod gguf;
pub mod scanner;

pub use gguf::{parse_gguf, GgufMetadata, QuantizationType};
pub use scanner::{LocalModel, ModelScanner};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator scanner`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/discovery/
git commit -m "feat(orchestrator): add model directory scanner"
```

---

## Chunk 2: Resource Management

### Task 5: Create DeviceId and resource types

**Files:**
- Create: `crates/river-orchestrator/src/resources/mod.rs`
- Create: `crates/river-orchestrator/src/resources/device.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create device types**

Create `crates/river-orchestrator/src/resources/device.rs`:

```rust
//! Device identification and resource tracking

use serde::Serialize;
use std::collections::HashMap;

/// Device identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(into = "String")]
pub enum DeviceId {
    Gpu(u32),
    Cpu,
}

impl DeviceId {
    /// Serialize for JSON API responses
    pub fn to_api_string(&self) -> String {
        match self {
            DeviceId::Gpu(idx) => format!("gpu:{}", idx),
            DeviceId::Cpu => "cpu".to_string(),
        }
    }

    /// Parse from API string
    pub fn from_api_string(s: &str) -> Option<Self> {
        if s == "cpu" {
            Some(DeviceId::Cpu)
        } else if let Some(idx) = s.strip_prefix("gpu:") {
            idx.parse().ok().map(DeviceId::Gpu)
        } else {
            None
        }
    }
}

impl From<DeviceId> for String {
    fn from(device: DeviceId) -> String {
        device.to_api_string()
    }
}

/// Resources available on a device
#[derive(Debug)]
pub struct DeviceResources {
    pub device: DeviceId,
    pub total_memory: u64,
    pub reserved: u64,
    pub allocated: u64,
    pub allocations: HashMap<String, u64>,
}

impl DeviceResources {
    /// Create new device resources
    pub fn new(device: DeviceId, total_memory: u64, reserved: u64) -> Self {
        Self {
            device,
            total_memory,
            reserved,
            allocated: 0,
            allocations: HashMap::new(),
        }
    }

    /// Available memory after reservations and allocations
    pub fn available(&self) -> u64 {
        self.total_memory
            .saturating_sub(self.reserved)
            .saturating_sub(self.allocated)
    }

    /// Check if bytes can fit
    pub fn can_fit(&self, bytes: u64) -> bool {
        self.available() >= bytes
    }

    /// Allocate memory for a model
    pub fn allocate(&mut self, model_id: &str, bytes: u64) -> bool {
        if self.can_fit(bytes) {
            self.allocated += bytes;
            self.allocations.insert(model_id.to_string(), bytes);
            true
        } else {
            false
        }
    }

    /// Release memory from a model
    pub fn release(&mut self, model_id: &str) {
        if let Some(bytes) = self.allocations.remove(model_id) {
            self.allocated = self.allocated.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_serialization() {
        assert_eq!(DeviceId::Gpu(0).to_api_string(), "gpu:0");
        assert_eq!(DeviceId::Cpu.to_api_string(), "cpu");
    }

    #[test]
    fn test_device_id_parsing() {
        assert_eq!(DeviceId::from_api_string("gpu:0"), Some(DeviceId::Gpu(0)));
        assert_eq!(DeviceId::from_api_string("gpu:1"), Some(DeviceId::Gpu(1)));
        assert_eq!(DeviceId::from_api_string("cpu"), Some(DeviceId::Cpu));
        assert_eq!(DeviceId::from_api_string("invalid"), None);
    }

    #[test]
    fn test_device_resources_allocation() {
        let mut res = DeviceResources::new(DeviceId::Gpu(0), 24_000_000_000, 500_000_000);

        // Available = 24GB - 500MB = 23.5GB
        assert_eq!(res.available(), 23_500_000_000);
        assert!(res.can_fit(20_000_000_000));

        // Allocate 20GB
        assert!(res.allocate("model1", 20_000_000_000));
        assert_eq!(res.available(), 3_500_000_000);

        // Can't fit another 5GB
        assert!(!res.can_fit(5_000_000_000));

        // Release
        res.release("model1");
        assert_eq!(res.available(), 23_500_000_000);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-orchestrator device`
Expected: 3 tests pass

- [ ] **Step 3: Create resources module**

Create `crates/river-orchestrator/src/resources/mod.rs`:

```rust
//! Resource management for GPU and CPU

pub mod device;

pub use device::{DeviceId, DeviceResources};
```

- [ ] **Step 4: Add resources module to lib.rs**

Modify `crates/river-orchestrator/src/lib.rs`, add after `pub mod discovery;`:

```rust
pub mod resources;
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/river-orchestrator/src/resources/
git add crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add device ID and resource types"
```

---

### Task 6: Add GPU discovery

**Files:**
- Create: `crates/river-orchestrator/src/resources/gpu.rs`
- Modify: `crates/river-orchestrator/src/resources/mod.rs`

- [ ] **Step 1: Create GPU discovery**

Create `crates/river-orchestrator/src/resources/gpu.rs`:

```rust
//! GPU discovery

use super::DeviceId;
use std::process::Command;

/// Information about a detected GPU
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub id: u32,
    pub name: String,
    pub total_vram: u64,
}

/// Discover available GPUs
pub fn detect_gpus() -> Vec<GpuInfo> {
    // Try NVIDIA first
    if let Some(gpus) = detect_nvidia_gpus() {
        if !gpus.is_empty() {
            return gpus;
        }
    }

    // TODO: Add AMD GPU detection via sysfs
    tracing::info!("No GPUs detected, will use CPU inference only");
    vec![]
}

fn detect_nvidia_gpus() -> Option<Vec<GpuInfo>> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=index,name,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let gpus: Vec<GpuInfo> = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(", ").collect();
            if parts.len() >= 3 {
                let id = parts[0].trim().parse().ok()?;
                let name = parts[1].trim().to_string();
                // nvidia-smi reports in MiB
                let vram_mib: u64 = parts[2].trim().parse().ok()?;
                let total_vram = vram_mib * 1024 * 1024;
                Some(GpuInfo { id, name, total_vram })
            } else {
                None
            }
        })
        .collect();

    Some(gpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_gpus_does_not_panic() {
        // This test just ensures detection doesn't panic
        // Actual GPU availability depends on system
        let _gpus = detect_gpus();
    }
}
```

- [ ] **Step 2: Update resources mod.rs**

Replace `crates/river-orchestrator/src/resources/mod.rs`:

```rust
//! Resource management for GPU and CPU

pub mod device;
pub mod gpu;

pub use device::{DeviceId, DeviceResources};
pub use gpu::{detect_gpus, GpuInfo};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator detect_gpus`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/resources/
git commit -m "feat(orchestrator): add GPU discovery via nvidia-smi"
```

---

### Task 7: Add system memory tracking with swap detection

**Files:**
- Create: `crates/river-orchestrator/src/resources/memory.rs`
- Modify: `crates/river-orchestrator/src/resources/mod.rs`

- [ ] **Step 1: Create memory tracking**

Create `crates/river-orchestrator/src/resources/memory.rs`:

```rust
//! System memory tracking

use std::fs;

/// System memory information including swap
#[derive(Debug, Clone)]
pub struct SystemMemory {
    pub total_ram: u64,
    pub available_ram: u64,
    pub total_swap: u64,
    pub available_swap: u64,
}

impl SystemMemory {
    /// Read current system memory from /proc/meminfo
    pub fn current() -> Self {
        Self::parse_meminfo().unwrap_or_else(|| Self {
            total_ram: 0,
            available_ram: 0,
            total_swap: 0,
            available_swap: 0,
        })
    }

    fn parse_meminfo() -> Option<Self> {
        let content = fs::read_to_string("/proc/meminfo").ok()?;

        let mut total_ram = 0u64;
        let mut available_ram = 0u64;
        let mut total_swap = 0u64;
        let mut free_swap = 0u64;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let value: u64 = parts[1].parse().unwrap_or(0) * 1024; // kB to bytes
                match parts[0] {
                    "MemTotal:" => total_ram = value,
                    "MemAvailable:" => available_ram = value,
                    "SwapTotal:" => total_swap = value,
                    "SwapFree:" => free_swap = value,
                    _ => {}
                }
            }
        }

        Some(Self {
            total_ram,
            available_ram,
            total_swap,
            available_swap: free_swap,
        })
    }

    /// Check if loading a model would require swap
    pub fn would_use_swap(&self, model_bytes: u64, used_by_models: u64) -> bool {
        let after_load = used_by_models + model_bytes;
        after_load > self.available_ram
    }

    /// Estimate how much swap would be used
    pub fn estimated_swap_usage(&self, model_bytes: u64, used_by_models: u64) -> u64 {
        let after_load = used_by_models + model_bytes;
        after_load.saturating_sub(self.available_ram)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_memory_current() {
        let mem = SystemMemory::current();
        // On Linux, should have positive values
        // On other systems, may be zero (graceful fallback)
        #[cfg(target_os = "linux")]
        {
            assert!(mem.total_ram > 0);
        }
    }

    #[test]
    fn test_swap_detection() {
        let mem = SystemMemory {
            total_ram: 64_000_000_000,      // 64GB
            available_ram: 32_000_000_000,  // 32GB available
            total_swap: 32_000_000_000,     // 32GB swap
            available_swap: 32_000_000_000,
        };

        // 20GB model with 10GB already used = 30GB total, fits in 32GB
        assert!(!mem.would_use_swap(20_000_000_000, 10_000_000_000));

        // 30GB model with 10GB already used = 40GB total, needs 8GB swap
        assert!(mem.would_use_swap(30_000_000_000, 10_000_000_000));
        assert_eq!(mem.estimated_swap_usage(30_000_000_000, 10_000_000_000), 8_000_000_000);
    }
}
```

- [ ] **Step 2: Update resources mod.rs**

Replace `crates/river-orchestrator/src/resources/mod.rs`:

```rust
//! Resource management for GPU and CPU

pub mod device;
pub mod gpu;
pub mod memory;

pub use device::{DeviceId, DeviceResources};
pub use gpu::{detect_gpus, GpuInfo};
pub use memory::SystemMemory;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator memory`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/resources/
git commit -m "feat(orchestrator): add system memory tracking with swap detection"
```

---

### Task 8: Create ResourceTracker

**Files:**
- Create: `crates/river-orchestrator/src/resources/tracker.rs`
- Modify: `crates/river-orchestrator/src/resources/mod.rs`

- [ ] **Step 1: Create resource tracker**

Create `crates/river-orchestrator/src/resources/tracker.rs`:

```rust
//! Central resource tracking

use super::{detect_gpus, DeviceId, DeviceResources, SystemMemory};
use tokio::sync::RwLock;

/// Configuration for resource tracking
#[derive(Debug, Clone)]
pub struct ResourceConfig {
    pub reserve_vram_bytes: u64,
    pub reserve_ram_bytes: u64,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            reserve_vram_bytes: 500 * 1024 * 1024,  // 500MB
            reserve_ram_bytes: 2 * 1024 * 1024 * 1024, // 2GB
        }
    }
}

/// Central tracker for all device resources
pub struct ResourceTracker {
    devices: RwLock<Vec<DeviceResources>>,
    config: ResourceConfig,
}

impl ResourceTracker {
    /// Initialize tracker by detecting available devices
    pub fn new(config: ResourceConfig) -> Self {
        let mut devices = Vec::new();

        // Detect GPUs
        for gpu in detect_gpus() {
            tracing::info!(
                "Detected GPU {}: {} with {:.1}GB VRAM",
                gpu.id,
                gpu.name,
                gpu.total_vram as f64 / 1_073_741_824.0
            );
            devices.push(DeviceResources::new(
                DeviceId::Gpu(gpu.id),
                gpu.total_vram,
                config.reserve_vram_bytes,
            ));
        }

        // Add CPU as fallback
        let sys_mem = SystemMemory::current();
        tracing::info!(
            "System RAM: {:.1}GB total, {:.1}GB available",
            sys_mem.total_ram as f64 / 1_073_741_824.0,
            sys_mem.available_ram as f64 / 1_073_741_824.0
        );
        devices.push(DeviceResources::new(
            DeviceId::Cpu,
            sys_mem.total_ram,
            config.reserve_ram_bytes,
        ));

        Self {
            devices: RwLock::new(devices),
            config,
        }
    }

    /// Find a device that can fit the required memory
    pub async fn find_device_for(&self, bytes_needed: u64) -> Option<DeviceId> {
        let devices = self.devices.read().await;

        // Try GPUs first (faster inference)
        for dev in devices.iter() {
            if matches!(dev.device, DeviceId::Gpu(_)) && dev.can_fit(bytes_needed) {
                return Some(dev.device);
            }
        }

        // Fall back to CPU
        for dev in devices.iter() {
            if matches!(dev.device, DeviceId::Cpu) && dev.can_fit(bytes_needed) {
                return Some(dev.device);
            }
        }

        None
    }

    /// Allocate memory on a device
    pub async fn allocate(&self, model_id: &str, device: DeviceId, bytes: u64) -> bool {
        let mut devices = self.devices.write().await;
        for dev in devices.iter_mut() {
            if dev.device == device {
                return dev.allocate(model_id, bytes);
            }
        }
        false
    }

    /// Release memory from a device
    pub async fn release(&self, model_id: &str, device: DeviceId) {
        let mut devices = self.devices.write().await;
        for dev in devices.iter_mut() {
            if dev.device == device {
                dev.release(model_id);
                return;
            }
        }
    }

    /// Get memory allocated on CPU for swap checking
    pub async fn cpu_allocated(&self) -> u64 {
        let devices = self.devices.read().await;
        for dev in devices.iter() {
            if matches!(dev.device, DeviceId::Cpu) {
                return dev.allocated;
            }
        }
        0
    }

    /// Get all device resources for API
    pub async fn get_all_resources(&self) -> Vec<DeviceResourcesSnapshot> {
        let devices = self.devices.read().await;
        devices.iter().map(DeviceResourcesSnapshot::from).collect()
    }
}

/// Snapshot of device resources for API responses
#[derive(Debug, Clone)]
pub struct DeviceResourcesSnapshot {
    pub device: DeviceId,
    pub total_memory: u64,
    pub allocated: u64,
    pub available: u64,
    pub allocations: Vec<(String, u64)>,
}

impl From<&DeviceResources> for DeviceResourcesSnapshot {
    fn from(dev: &DeviceResources) -> Self {
        Self {
            device: dev.device,
            total_memory: dev.total_memory,
            allocated: dev.allocated,
            available: dev.available(),
            allocations: dev.allocations.iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resource_tracker_creation() {
        let tracker = ResourceTracker::new(ResourceConfig::default());
        // Should at least have CPU
        let resources = tracker.get_all_resources().await;
        assert!(!resources.is_empty());
    }

    #[tokio::test]
    async fn test_resource_allocation() {
        let tracker = ResourceTracker::new(ResourceConfig::default());

        // Allocate on CPU (always available)
        let allocated = tracker.allocate("test-model", DeviceId::Cpu, 1_000_000_000).await;
        assert!(allocated);

        assert_eq!(tracker.cpu_allocated().await, 1_000_000_000);

        tracker.release("test-model", DeviceId::Cpu).await;
        assert_eq!(tracker.cpu_allocated().await, 0);
    }
}
```

- [ ] **Step 2: Update resources mod.rs**

Replace `crates/river-orchestrator/src/resources/mod.rs`:

```rust
//! Resource management for GPU and CPU

pub mod device;
pub mod gpu;
pub mod memory;
pub mod tracker;

pub use device::{DeviceId, DeviceResources};
pub use gpu::{detect_gpus, GpuInfo};
pub use memory::SystemMemory;
pub use tracker::{ResourceConfig, ResourceTracker, DeviceResourcesSnapshot};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator tracker`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/resources/
git commit -m "feat(orchestrator): add central resource tracker"
```

---

## Chunk 3: Process Lifecycle Management

### Task 9: Create PortAllocator

**Files:**
- Create: `crates/river-orchestrator/src/process/mod.rs`
- Create: `crates/river-orchestrator/src/process/port.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create port allocator**

Create `crates/river-orchestrator/src/process/port.rs`:

```rust
//! Port allocation for llama-server instances

use river_core::RiverError;
use std::collections::HashSet;

/// Allocates ports from a configured range
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
    allocated: HashSet<u16>,
}

impl PortAllocator {
    /// Create a new port allocator with the given range
    pub fn new(range_start: u16, range_end: u16) -> Self {
        Self {
            range_start,
            range_end,
            allocated: HashSet::new(),
        }
    }

    /// Allocate the next available port
    pub fn next(&mut self) -> Result<u16, RiverError> {
        for port in self.range_start..=self.range_end {
            if !self.allocated.contains(&port) {
                self.allocated.insert(port);
                return Ok(port);
            }
        }
        Err(RiverError::orchestrator(format!(
            "No available ports in range {}-{}",
            self.range_start, self.range_end
        )))
    }

    /// Release a port back to the pool
    pub fn release(&mut self, port: u16) {
        self.allocated.remove(&port);
    }

    /// Check how many ports are allocated
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_allocation() {
        let mut allocator = PortAllocator::new(8080, 8082);

        assert_eq!(allocator.next().unwrap(), 8080);
        assert_eq!(allocator.next().unwrap(), 8081);
        assert_eq!(allocator.next().unwrap(), 8082);
        assert!(allocator.next().is_err());
    }

    #[test]
    fn test_port_release() {
        let mut allocator = PortAllocator::new(8080, 8080);

        let port = allocator.next().unwrap();
        assert!(allocator.next().is_err());

        allocator.release(port);
        assert_eq!(allocator.next().unwrap(), 8080);
    }

    #[test]
    fn test_allocated_count() {
        let mut allocator = PortAllocator::new(8080, 8085);

        assert_eq!(allocator.allocated_count(), 0);
        allocator.next().unwrap();
        allocator.next().unwrap();
        assert_eq!(allocator.allocated_count(), 2);
    }
}
```

- [ ] **Step 2: Create process module**

Create `crates/river-orchestrator/src/process/mod.rs`:

```rust
//! Process lifecycle management for llama-server

pub mod port;

pub use port::PortAllocator;
```

- [ ] **Step 3: Add process module to lib.rs**

Modify `crates/river-orchestrator/src/lib.rs`, add after `pub mod resources;`:

```rust
pub mod process;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator port`
Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/process/
git add crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add port allocator for llama-server instances"
```

---

### Task 10: Create ProcessManager

**Files:**
- Create: `crates/river-orchestrator/src/process/manager.rs`
- Modify: `crates/river-orchestrator/src/process/mod.rs`

- [ ] **Step 1: Create process info types**

Create `crates/river-orchestrator/src/process/manager.rs`:

```rust
//! Process management for llama-server instances

use super::PortAllocator;
use crate::discovery::LocalModel;
use crate::resources::DeviceId;
use river_core::RiverError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

/// Health state of a process
#[derive(Debug, Clone)]
pub enum ProcessHealth {
    Starting,
    Healthy,
    Unhealthy { since: Instant, reason: String },
    Dead,
}

/// Information about a running llama-server process
pub struct ProcessInfo {
    pub model_id: String,
    pub pid: u32,
    pub port: u16,
    pub device: DeviceId,
    pub started_at: Instant,
    pub last_request: Instant,
    pub health: ProcessHealth,
    child: Child,
}

impl ProcessInfo {
    /// Get idle time in seconds
    pub fn idle_seconds(&self) -> u64 {
        self.last_request.elapsed().as_secs()
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Record a request was made
    pub fn record_request(&mut self) {
        self.last_request = Instant::now();
    }
}

/// Configuration for process manager
#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub llama_server_path: PathBuf,
    pub port_range_start: u16,
    pub port_range_end: u16,
    pub default_ctx_size: u32,
    pub health_check_timeout: Duration,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            llama_server_path: PathBuf::from("llama-server"),
            port_range_start: 8080,
            port_range_end: 8180,
            default_ctx_size: 8192,
            health_check_timeout: Duration::from_secs(5),
        }
    }
}

/// Manager for llama-server processes
pub struct ProcessManager {
    processes: RwLock<HashMap<String, ProcessInfo>>,
    port_allocator: RwLock<PortAllocator>,
    llama_server_path: Option<PathBuf>,
    config: ProcessConfig,
}

impl ProcessManager {
    /// Create a new process manager
    pub fn new(config: ProcessConfig) -> Self {
        let llama_server_path = Self::find_llama_server(&config.llama_server_path);

        if llama_server_path.is_none() {
            tracing::warn!(
                "llama-server not found at '{}'. Local model inference unavailable.",
                config.llama_server_path.display()
            );
        }

        let port_allocator = PortAllocator::new(
            config.port_range_start,
            config.port_range_end,
        );

        Self {
            processes: RwLock::new(HashMap::new()),
            port_allocator: RwLock::new(port_allocator),
            llama_server_path,
            config,
        }
    }

    fn find_llama_server(configured_path: &PathBuf) -> Option<PathBuf> {
        if configured_path.exists() {
            return Some(configured_path.clone());
        }

        // Try PATH lookup
        which::which("llama-server").ok()
    }

    /// Check if llama-server is available
    pub fn is_available(&self) -> bool {
        self.llama_server_path.is_some()
    }

    /// Spawn a new llama-server process
    pub async fn spawn(&self, model: &LocalModel, device: DeviceId) -> Result<u16, RiverError> {
        let llama_server = self.llama_server_path.as_ref().ok_or_else(|| {
            RiverError::orchestrator("Local model inference unavailable: llama-server not found")
        })?;

        let port = {
            let mut allocator = self.port_allocator.write().await;
            allocator.next()?
        };

        let mut cmd = Command::new(llama_server);
        cmd.arg("--model").arg(&model.path)
           .arg("--port").arg(port.to_string())
           .arg("--ctx-size").arg(model.metadata.context_length.to_string());

        // Device-specific args
        match device {
            DeviceId::Gpu(idx) => {
                cmd.arg("--n-gpu-layers").arg("-1");
                cmd.env("CUDA_VISIBLE_DEVICES", idx.to_string());
            }
            DeviceId::Cpu => {
                cmd.arg("--n-gpu-layers").arg("0");
            }
        }

        // Suppress stdout/stderr to avoid blocking
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        tracing::info!(
            "Spawning llama-server for {} on {} (port {})",
            model.id,
            device.to_api_string(),
            port
        );

        let child = cmd.spawn().map_err(|e| {
            RiverError::orchestrator(format!("Failed to spawn llama-server: {}", e))
        })?;

        let pid = child.id().unwrap_or(0);
        let now = Instant::now();

        let info = ProcessInfo {
            model_id: model.id.clone(),
            pid,
            port,
            device,
            started_at: now,
            last_request: now,
            health: ProcessHealth::Starting,
            child,
        };

        {
            let mut processes = self.processes.write().await;
            processes.insert(model.id.clone(), info);
        }

        // Wait for health check
        self.wait_for_ready(&model.id, port).await?;

        Ok(port)
    }

    async fn wait_for_ready(&self, model_id: &str, port: u16) -> Result<(), RiverError> {
        let client = reqwest::Client::builder()
            .timeout(self.config.health_check_timeout)
            .build()
            .map_err(|e| RiverError::orchestrator(format!("Failed to create HTTP client: {}", e)))?;

        let url = format!("http://127.0.0.1:{}/health", port);
        let start = Instant::now();
        let max_wait = Duration::from_secs(120);

        while start.elapsed() < max_wait {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    // Mark as healthy
                    let mut processes = self.processes.write().await;
                    if let Some(proc) = processes.get_mut(model_id) {
                        proc.health = ProcessHealth::Healthy;
                    }
                    tracing::info!("llama-server for {} is ready", model_id);
                    return Ok(());
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }

        // Timeout - kill process
        self.kill(model_id).await;
        Err(RiverError::orchestrator(format!(
            "llama-server for {} failed to become ready within 120s",
            model_id
        )))
    }

    /// Kill a process
    pub async fn kill(&self, model_id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(mut info) = processes.remove(model_id) {
            tracing::info!("Killing llama-server for {}", model_id);
            let _ = info.child.kill().await;

            let mut allocator = self.port_allocator.write().await;
            allocator.release(info.port);
        }
    }

    /// Get process info for a model
    pub async fn get_process(&self, model_id: &str) -> Option<ProcessSnapshot> {
        let processes = self.processes.read().await;
        processes.get(model_id).map(ProcessSnapshot::from)
    }

    /// Get all running processes
    pub async fn get_all_processes(&self) -> Vec<ProcessSnapshot> {
        let processes = self.processes.read().await;
        processes.values().map(ProcessSnapshot::from).collect()
    }

    /// Record a request was made to a model
    pub async fn record_request(&self, model_id: &str) {
        let mut processes = self.processes.write().await;
        if let Some(proc) = processes.get_mut(model_id) {
            proc.record_request();
        }
    }

    /// Get model IDs that have been idle for longer than threshold
    pub async fn idle_models(&self, threshold: Duration) -> Vec<String> {
        let processes = self.processes.read().await;
        processes
            .iter()
            .filter(|(_, proc)| proc.last_request.elapsed() > threshold)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Snapshot of process info for API responses
#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub model_id: String,
    pub pid: u32,
    pub port: u16,
    pub device: DeviceId,
    pub uptime_seconds: u64,
    pub idle_seconds: u64,
    pub healthy: bool,
}

impl From<&ProcessInfo> for ProcessSnapshot {
    fn from(info: &ProcessInfo) -> Self {
        Self {
            model_id: info.model_id.clone(),
            pid: info.pid,
            port: info.port,
            device: info.device,
            uptime_seconds: info.uptime_seconds(),
            idle_seconds: info.idle_seconds(),
            healthy: matches!(info.health, ProcessHealth::Healthy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_config_default() {
        let config = ProcessConfig::default();
        assert_eq!(config.port_range_start, 8080);
        assert_eq!(config.port_range_end, 8180);
    }

    #[tokio::test]
    async fn test_process_manager_creation() {
        let manager = ProcessManager::new(ProcessConfig::default());
        // May or may not be available depending on system
        let _available = manager.is_available();
    }
}
```

- [ ] **Step 2: Update process mod.rs**

Replace `crates/river-orchestrator/src/process/mod.rs`:

```rust
//! Process lifecycle management for llama-server

pub mod manager;
pub mod port;

pub use manager::{ProcessConfig, ProcessManager, ProcessSnapshot};
pub use port::PortAllocator;
```

- [ ] **Step 3: Add reqwest dependency**

The workspace already has reqwest, verify it's accessible:

Run: `cargo check -p river-orchestrator`
Expected: Compiles (reqwest is workspace dep)

If not, add to `crates/river-orchestrator/Cargo.toml`:
```toml
reqwest.workspace = true
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator manager`
Expected: 2 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/process/
git add crates/river-orchestrator/Cargo.toml
git commit -m "feat(orchestrator): add process manager for llama-server lifecycle"
```

---

### Task 10b: Add health monitoring background loop

**Files:**
- Create: `crates/river-orchestrator/src/process/health.rs`
- Modify: `crates/river-orchestrator/src/process/mod.rs`

- [ ] **Step 1: Create health monitoring module**

Create `crates/river-orchestrator/src/process/health.rs`:

```rust
//! Background health monitoring for llama-server processes

use super::ProcessManager;
use std::sync::Arc;
use std::time::Duration;

/// Run health check loop in background
pub async fn health_check_loop(manager: Arc<ProcessManager>, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;

        // Get all model IDs to check
        let model_ids: Vec<String> = manager.get_all_processes().await
            .into_iter()
            .map(|p| p.model_id)
            .collect();

        for model_id in model_ids {
            if let Some(snapshot) = manager.get_process(&model_id).await {
                let healthy = check_endpoint_health(snapshot.port).await;
                if !healthy {
                    manager.mark_unhealthy(&model_id, "Health check failed").await;
                }
            }
        }
    }
}

async fn check_endpoint_health(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let url = format!("http://127.0.0.1:{}/health", port);
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
```

- [ ] **Step 2: Add mark_unhealthy to ProcessManager**

Add to `crates/river-orchestrator/src/process/manager.rs` in `impl ProcessManager`:

```rust
    /// Mark a process as unhealthy
    pub async fn mark_unhealthy(&self, model_id: &str, reason: &str) {
        let mut processes = self.processes.write().await;
        if let Some(proc) = processes.get_mut(model_id) {
            proc.health = ProcessHealth::Unhealthy {
                since: std::time::Instant::now(),
                reason: reason.to_string(),
            };
            tracing::warn!("Process {} marked unhealthy: {}", model_id, reason);
        }
    }
```

- [ ] **Step 3: Update process mod.rs**

Replace `crates/river-orchestrator/src/process/mod.rs`:

```rust
//! Process lifecycle management for llama-server

pub mod health;
pub mod manager;
pub mod port;

pub use health::health_check_loop;
pub use manager::{ProcessConfig, ProcessManager, ProcessSnapshot};
pub use port::PortAllocator;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p river-orchestrator process`
Expected: All process tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/river-orchestrator/src/process/
git commit -m "feat(orchestrator): add background health monitoring loop"
```

---

## Chunk 4: Extended State and LiteLLM Integration

### Task 11: Create ExternalModel type

**Files:**
- Create: `crates/river-orchestrator/src/external.rs`
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Create external model types**

Create `crates/river-orchestrator/src/external.rs`:

```rust
//! External model configuration (LiteLLM)

use serde::{Deserialize, Serialize};

/// An external model accessible via LiteLLM or similar proxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalModel {
    pub id: String,
    pub provider: String,
    pub litellm_model: String,
    pub api_base: String,
}

impl ExternalModel {
    /// Get the chat completions endpoint
    pub fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.api_base)
    }
}

/// External models configuration file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalModelsFile {
    pub external_models: Vec<ExternalModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_model_endpoint() {
        let model = ExternalModel {
            id: "claude-sonnet".to_string(),
            provider: "litellm".to_string(),
            litellm_model: "claude-sonnet-4-20250514".to_string(),
            api_base: "http://localhost:4000".to_string(),
        };
        assert_eq!(model.endpoint(), "http://localhost:4000/v1/chat/completions");
    }

    #[test]
    fn test_external_models_deserialize() {
        let json = r#"{
            "external_models": [
                {
                    "id": "claude-sonnet",
                    "provider": "litellm",
                    "litellm_model": "claude-sonnet-4-20250514",
                    "api_base": "http://localhost:4000"
                }
            ]
        }"#;
        let file: ExternalModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.external_models.len(), 1);
        assert_eq!(file.external_models[0].id, "claude-sonnet");
    }
}
```

- [ ] **Step 2: Add to lib.rs**

Add to `crates/river-orchestrator/src/lib.rs` after `pub mod process;`:

```rust
pub mod external;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p river-orchestrator external`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/external.rs
git add crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): add external model types for LiteLLM"
```

---

### Task 12: Extend OrchestratorState with new fields

**Files:**
- Modify: `crates/river-orchestrator/src/state.rs`

- [ ] **Step 1: Add imports and new fields to state**

Replace `crates/river-orchestrator/src/state.rs`:

```rust
//! Shared application state

use crate::agents::{AgentInfo, AgentStatus};
use crate::config::OrchestratorConfig;
use crate::discovery::{LocalModel, ModelScanner};
use crate::external::ExternalModel;
use crate::models::ModelInfo;
use crate::process::{ProcessConfig, ProcessManager, ProcessSnapshot};
use crate::resources::{DeviceId, ResourceConfig, ResourceTracker, SystemMemory};
use river_core::RiverError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Model status for API responses
#[derive(Debug, Clone)]
pub enum LocalModelStatus {
    Available,
    Loading,
    Loaded {
        endpoint: String,
        device: DeviceId,
        idle_seconds: u64,
    },
    Error(String),
}

/// Extended local model with runtime status
#[derive(Debug, Clone)]
pub struct LocalModelEntry {
    pub model: LocalModel,
    pub status: LocalModelStatus,
    pub releasable: bool,  // Can be evicted if resources needed
}

/// Shared orchestrator state
pub struct OrchestratorState {
    // Existing fields
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub models: Vec<ModelInfo>,  // Legacy static models
    pub config: OrchestratorConfig,

    // New fields for advanced orchestrator
    pub local_models: RwLock<HashMap<String, LocalModelEntry>>,
    pub external_models: Vec<ExternalModel>,
    pub resource_tracker: Arc<ResourceTracker>,
    pub process_manager: Arc<ProcessManager>,
}

impl OrchestratorState {
    /// Create new orchestrator state (legacy)
    pub fn new(config: OrchestratorConfig, models: Vec<ModelInfo>) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            models,
            config,
            local_models: RwLock::new(HashMap::new()),
            external_models: Vec::new(),
            resource_tracker: Arc::new(ResourceTracker::new(ResourceConfig::default())),
            process_manager: Arc::new(ProcessManager::new(ProcessConfig::default())),
        }
    }

    /// Create new orchestrator state with advanced features
    pub fn new_advanced(
        config: OrchestratorConfig,
        local_models: Vec<LocalModel>,
        external_models: Vec<ExternalModel>,
        resource_config: ResourceConfig,
        process_config: ProcessConfig,
    ) -> Self {
        let local_entries: HashMap<String, LocalModelEntry> = local_models
            .into_iter()
            .map(|m| {
                let id = m.id.clone();
                let entry = LocalModelEntry {
                    model: m,
                    status: LocalModelStatus::Available,
                    releasable: false,
                };
                (id, entry)
            })
            .collect();

        Self {
            agents: RwLock::new(HashMap::new()),
            models: Vec::new(),
            config,
            local_models: RwLock::new(local_entries),
            external_models,
            resource_tracker: Arc::new(ResourceTracker::new(resource_config)),
            process_manager: Arc::new(ProcessManager::new(process_config)),
        }
    }

    /// Get health threshold as Duration
    pub fn health_threshold(&self) -> Duration {
        Duration::from_secs(self.config.health_threshold_seconds)
    }

    /// Register or update agent heartbeat
    pub async fn heartbeat(&self, name: String, gateway_url: String) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&name) {
            agent.heartbeat();
            if agent.gateway_url != gateway_url {
                agent.update_url(gateway_url);
            }
        } else {
            agents.insert(name.clone(), AgentInfo::new(name, gateway_url));
        }
    }

    /// Get all agent statuses
    pub async fn agent_statuses(&self) -> Vec<AgentStatus> {
        let agents = self.agents.read().await;
        let threshold = self.health_threshold();
        agents
            .values()
            .map(|a| AgentStatus::from_agent(a, threshold))
            .collect()
    }

    /// Get count of registered agents
    pub async fn agent_count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Request a model to be loaded
    pub async fn request_model(&self, model_id: &str) -> Result<ModelRequestResponse, RiverError> {
        // Check external models first
        for ext in &self.external_models {
            if ext.id == model_id {
                return Ok(ModelRequestResponse::Ready {
                    endpoint: ext.endpoint(),
                    device: None,
                    warning: None,
                });
            }
        }

        // Check local models
        let mut local_models = self.local_models.write().await;
        let entry = local_models.get_mut(model_id).ok_or_else(|| {
            RiverError::orchestrator(format!("Model not found: {}", model_id))
        })?;

        // Already loaded?
        if let LocalModelStatus::Loaded { endpoint, device, idle_seconds } = &entry.status {
            return Ok(ModelRequestResponse::Ready {
                endpoint: endpoint.clone(),
                device: Some(*device),
                warning: None,
            });
        }

        // Check if llama-server is available
        if !self.process_manager.is_available() {
            return Err(RiverError::orchestrator(
                "Local model inference unavailable: llama-server not found"
            ));
        }

        // Find a device (or evict to make space)
        let vram_needed = entry.model.metadata.estimate_vram();
        let device = match self.resource_tracker.find_device_for(vram_needed).await {
            Some(dev) => dev,
            None => {
                // Try to evict releasable models to make space
                self.evict_for_space(vram_needed).await?;
                self.resource_tracker.find_device_for(vram_needed).await
                    .ok_or_else(|| {
                        RiverError::orchestrator(format!(
                            "Insufficient resources: model requires {} bytes, eviction failed",
                            vram_needed
                        ))
                    })?
            }
        };

        // Check for swap warning on CPU
        let warning = if matches!(device, DeviceId::Cpu) {
            let sys_mem = SystemMemory::current();
            let cpu_allocated = self.resource_tracker.cpu_allocated().await;
            if sys_mem.would_use_swap(vram_needed, cpu_allocated) {
                let swap_gb = sys_mem.estimated_swap_usage(vram_needed, cpu_allocated) as f64
                    / 1_073_741_824.0;
                Some(format!(
                    "Model will use ~{:.1}GB swap. Expect slow inference due to memory pressure.",
                    swap_gb
                ))
            } else {
                None
            }
        } else {
            None
        };

        // Mark as loading
        entry.status = LocalModelStatus::Loading;

        // Spawn process
        let port = self.process_manager.spawn(&entry.model, device).await?;

        // Allocate resources
        self.resource_tracker.allocate(model_id, device, vram_needed).await;

        // Update status
        let endpoint = format!("http://127.0.0.1:{}/v1/chat/completions", port);
        entry.status = LocalModelStatus::Loaded {
            endpoint: endpoint.clone(),
            device,
            idle_seconds: 0,
        };

        Ok(ModelRequestResponse::Ready {
            endpoint,
            device: Some(device),
            warning,
        })
    }

    /// Mark a model as releasable for eviction
    pub async fn release_model(&self, model_id: &str) -> bool {
        let mut local_models = self.local_models.write().await;
        if let Some(entry) = local_models.get_mut(model_id) {
            entry.releasable = true;
            true
        } else {
            false
        }
    }

    /// Evict releasable models to free up space
    async fn evict_for_space(&self, bytes_needed: u64) -> Result<(), RiverError> {
        // Get releasable models sorted by idle time (oldest first)
        let candidates: Vec<(String, u64)> = {
            let local_models = self.local_models.read().await;
            let mut list: Vec<_> = local_models
                .iter()
                .filter(|(_, entry)| entry.releasable)
                .filter_map(|(id, entry)| {
                    if let LocalModelStatus::Loaded { .. } = &entry.status {
                        Some((id.clone(), entry.model.metadata.estimate_vram()))
                    } else {
                        None
                    }
                })
                .collect();
            // Sort by VRAM (largest first for efficient eviction)
            list.sort_by(|a, b| b.1.cmp(&a.1));
            list
        };

        let mut freed = 0u64;
        for (model_id, vram) in candidates {
            if freed >= bytes_needed {
                break;
            }
            tracing::info!("Evicting releasable model {} to free space", model_id);
            self.unload_model(&model_id).await?;
            freed += vram;
        }

        if freed >= bytes_needed {
            Ok(())
        } else {
            Err(RiverError::orchestrator(format!(
                "Could not free enough space: needed {} bytes, freed {} bytes",
                bytes_needed, freed
            )))
        }
    }

    /// Unload a model
    pub async fn unload_model(&self, model_id: &str) -> Result<(), RiverError> {
        // Get device before unloading
        let device = {
            let local_models = self.local_models.read().await;
            if let Some(entry) = local_models.get(model_id) {
                match &entry.status {
                    LocalModelStatus::Loaded { device, .. } => Some(*device),
                    _ => None,
                }
            } else {
                None
            }
        };

        // Kill process
        self.process_manager.kill(model_id).await;

        // Release resources
        if let Some(device) = device {
            self.resource_tracker.release(model_id, device).await;
        }

        // Update status
        let mut local_models = self.local_models.write().await;
        if let Some(entry) = local_models.get_mut(model_id) {
            entry.status = LocalModelStatus::Available;
            entry.releasable = false;
        }

        Ok(())
    }

    /// Check if llama-server is available
    pub fn llama_server_available(&self) -> bool {
        self.process_manager.is_available()
    }
}

/// Response from model request
#[derive(Debug)]
pub enum ModelRequestResponse {
    Ready {
        endpoint: String,
        device: Option<DeviceId>,
        warning: Option<String>,
    },
    Loading {
        estimated_seconds: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let config = OrchestratorConfig::default();
        let state = OrchestratorState::new(config, vec![]);
        assert_eq!(state.config.port, 5000);
    }

    #[tokio::test]
    async fn test_state_heartbeat_creates_agent() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_state_heartbeat_updates_existing() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("test".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("test".to_string(), "http://localhost:4000".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].gateway_url, "http://localhost:4000");
    }

    #[tokio::test]
    async fn test_state_agent_statuses() {
        let state = OrchestratorState::new(OrchestratorConfig::default(), vec![]);
        state.heartbeat("agent1".to_string(), "http://localhost:3000".to_string()).await;
        state.heartbeat("agent2".to_string(), "http://localhost:3001".to_string()).await;

        let statuses = state.agent_statuses().await;
        assert_eq!(statuses.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-orchestrator state`
Expected: 4 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/src/state.rs
git commit -m "feat(orchestrator): extend state with model management and resources"
```

---

### Task 13: Add new API routes

**Files:**
- Modify: `crates/river-orchestrator/src/api/routes.rs`

- [ ] **Step 1: Add new route handlers and types**

Replace `crates/river-orchestrator/src/api/routes.rs`:

```rust
//! HTTP route handlers

use crate::agents::AgentStatus;
use crate::models::ModelInfo;
use crate::resources::DeviceId;
use crate::state::{LocalModelStatus, ModelRequestResponse, OrchestratorState};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health check response
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub agents_registered: usize,
}

/// Heartbeat request
#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub agent: String,
    pub gateway_url: String,
}

/// Heartbeat response
#[derive(Serialize)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
}

/// Model request
#[derive(Deserialize)]
pub struct ModelRequest {
    pub model: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
}

fn default_priority() -> String {
    "interactive".to_string()
}

fn default_timeout() -> u32 {
    120
}

/// Model request response
#[derive(Serialize)]
pub struct ModelRequestApiResponse {
    pub status: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Model release request
#[derive(Deserialize)]
pub struct ModelReleaseRequest {
    pub model: String,
}

/// Model release response
#[derive(Serialize)]
pub struct ModelReleaseResponse {
    pub acknowledged: bool,
}

/// Local model info for API
#[derive(Serialize)]
pub struct LocalModelApiResponse {
    pub id: String,
    pub path: String,
    pub architecture: String,
    pub parameters: String,
    pub quantization: String,
    pub estimated_vram_gb: f64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u64>,
}

/// External model info for API
#[derive(Serialize)]
pub struct ExternalModelApiResponse {
    pub id: String,
    pub provider: String,
    pub endpoint: String,
    pub status: String,
}

/// Device resource info for API
#[derive(Serialize)]
pub struct DeviceApiResponse {
    pub id: String,
    pub total_memory_gb: f64,
    pub used_memory_gb: f64,
    pub available_memory_gb: f64,
}

/// Models available response
#[derive(Serialize)]
pub struct ModelsAvailableResponse {
    pub local: Vec<LocalModelApiResponse>,
    pub external: Vec<ExternalModelApiResponse>,
    pub resources: ResourcesApiResponse,
    pub llama_server_available: bool,
}

/// Resources API response
#[derive(Serialize)]
pub struct ResourcesApiResponse {
    pub devices: Vec<DeviceApiResponse>,
    pub loaded_models: Vec<LoadedModelApiResponse>,
}

/// Loaded model info for resources endpoint
#[derive(Serialize)]
pub struct LoadedModelApiResponse {
    pub model_id: String,
    pub device: String,
    pub vram_bytes: u64,
    pub port: u16,
    pub pid: u32,
    pub uptime_seconds: u64,
    pub idle_seconds: u64,
}

/// Create the router with all routes
pub fn create_router(state: Arc<OrchestratorState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/heartbeat", post(handle_heartbeat))
        .route("/agents/status", get(agents_status))
        .route("/models/available", get(models_available))
        .route("/model/request", post(model_request))
        .route("/model/release", post(model_release))
        .route("/resources", get(resources))
        .with_state(state)
}

async fn health_check(State(state): State<Arc<OrchestratorState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        agents_registered: state.agent_count().await,
    })
}

async fn handle_heartbeat(
    State(state): State<Arc<OrchestratorState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Json<HeartbeatResponse> {
    tracing::debug!("Heartbeat from {} at {}", req.agent, req.gateway_url);
    state.heartbeat(req.agent, req.gateway_url).await;
    Json(HeartbeatResponse { acknowledged: true })
}

async fn agents_status(
    State(state): State<Arc<OrchestratorState>>,
) -> Json<Vec<AgentStatus>> {
    Json(state.agent_statuses().await)
}

async fn models_available(
    State(state): State<Arc<OrchestratorState>>,
) -> Json<ModelsAvailableResponse> {
    let local_models = state.local_models.read().await;

    let local: Vec<LocalModelApiResponse> = local_models
        .values()
        .map(|entry| {
            let (status, endpoint, device, idle_seconds) = match &entry.status {
                LocalModelStatus::Available => ("available".to_string(), None, None, None),
                LocalModelStatus::Loading => ("loading".to_string(), None, None, None),
                LocalModelStatus::Loaded { endpoint, device, idle_seconds } => (
                    "loaded".to_string(),
                    Some(endpoint.clone()),
                    Some(device.to_api_string()),
                    Some(*idle_seconds),
                ),
                LocalModelStatus::Error(e) => (format!("error: {}", e), None, None, None),
            };

            LocalModelApiResponse {
                id: entry.model.id.clone(),
                path: entry.model.path.display().to_string(),
                architecture: entry.model.metadata.architecture.clone(),
                parameters: format_parameters(entry.model.metadata.parameters),
                quantization: format!("{:?}", entry.model.metadata.quantization),
                estimated_vram_gb: entry.model.metadata.estimate_vram() as f64 / 1_073_741_824.0,
                status,
                endpoint,
                device,
                idle_seconds,
            }
        })
        .collect();

    let external: Vec<ExternalModelApiResponse> = state.external_models
        .iter()
        .map(|m| ExternalModelApiResponse {
            id: m.id.clone(),
            provider: m.provider.clone(),
            endpoint: m.endpoint(),
            status: "available".to_string(),
        })
        .collect();

    let device_resources = state.resource_tracker.get_all_resources().await;
    let devices: Vec<DeviceApiResponse> = device_resources
        .iter()
        .map(|d| DeviceApiResponse {
            id: d.device.to_api_string(),
            total_memory_gb: d.total_memory as f64 / 1_073_741_824.0,
            used_memory_gb: d.allocated as f64 / 1_073_741_824.0,
            available_memory_gb: d.available as f64 / 1_073_741_824.0,
        })
        .collect();

    Json(ModelsAvailableResponse {
        local,
        external,
        resources: ResourcesApiResponse { devices },
        llama_server_available: state.llama_server_available(),
    })
}

async fn model_request(
    State(state): State<Arc<OrchestratorState>>,
    Json(req): Json<ModelRequest>,
) -> Result<Json<ModelRequestApiResponse>, (StatusCode, Json<ModelRequestApiResponse>)> {
    match state.request_model(&req.model).await {
        Ok(ModelRequestResponse::Ready { endpoint, device, warning }) => {
            Ok(Json(ModelRequestApiResponse {
                status: "ready".to_string(),
                model: req.model,
                endpoint: Some(endpoint),
                device: device.map(|d| d.to_api_string()),
                warning,
                error: None,
            }))
        }
        Ok(ModelRequestResponse::Loading { estimated_seconds }) => {
            Ok(Json(ModelRequestApiResponse {
                status: "loading".to_string(),
                model: req.model,
                endpoint: None,
                device: None,
                warning: None,
                error: None,
            }))
        }
        Err(e) => {
            Err((
                StatusCode::BAD_REQUEST,
                Json(ModelRequestApiResponse {
                    status: "error".to_string(),
                    model: req.model,
                    endpoint: None,
                    device: None,
                    warning: None,
                    error: Some(e.to_string()),
                }),
            ))
        }
    }
}

async fn model_release(
    State(state): State<Arc<OrchestratorState>>,
    Json(req): Json<ModelReleaseRequest>,
) -> Json<ModelReleaseResponse> {
    let acknowledged = state.release_model(&req.model).await;
    Json(ModelReleaseResponse { acknowledged })
}

async fn resources(
    State(state): State<Arc<OrchestratorState>>,
) -> Json<ResourcesApiResponse> {
    let device_resources = state.resource_tracker.get_all_resources().await;
    let devices: Vec<DeviceApiResponse> = device_resources
        .iter()
        .map(|d| DeviceApiResponse {
            id: d.device.to_api_string(),
            total_memory_gb: d.total_memory as f64 / 1_073_741_824.0,
            used_memory_gb: d.allocated as f64 / 1_073_741_824.0,
            available_memory_gb: d.available as f64 / 1_073_741_824.0,
        })
        .collect();

    // Get loaded models from process manager
    let processes = state.process_manager.get_all_processes().await;
    let local_models = state.local_models.read().await;
    let loaded_models: Vec<LoadedModelApiResponse> = processes
        .iter()
        .map(|p| {
            let vram_bytes = local_models
                .get(&p.model_id)
                .map(|e| e.model.metadata.estimate_vram())
                .unwrap_or(0);
            LoadedModelApiResponse {
                model_id: p.model_id.clone(),
                device: p.device.to_api_string(),
                vram_bytes,
                port: p.port,
                pid: p.pid,
                uptime_seconds: p.uptime_seconds,
                idle_seconds: p.idle_seconds,
            }
        })
        .collect();

    Json(ResourcesApiResponse { devices, loaded_models })
}

fn format_parameters(params: u64) -> String {
    if params >= 1_000_000_000 {
        format!("{:.0}B", params as f64 / 1_000_000_000.0)
    } else if params >= 1_000_000 {
        format!("{:.0}M", params as f64 / 1_000_000.0)
    } else {
        format!("{}", params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OrchestratorConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<OrchestratorState> {
        Arc::new(OrchestratorState::new(OrchestratorConfig::default(), vec![]))
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_heartbeat() {
        let state = test_state();
        let app = create_router(state.clone());

        let body = serde_json::json!({
            "agent": "thomas",
            "gateway_url": "http://localhost:3000"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/heartbeat")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.agent_count().await, 1);
    }

    #[tokio::test]
    async fn test_models_available() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/models/available").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_resources() {
        let app = create_router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/resources").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-orchestrator routes`
Expected: 4 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/src/api/routes.rs
git commit -m "feat(orchestrator): add model request/release API endpoints"
```

---

## Chunk 5: CLI Updates and Integration

### Task 14: Update configuration types

**Files:**
- Modify: `crates/river-orchestrator/src/config.rs`

- [ ] **Step 1: Extend configuration**

Replace `crates/river-orchestrator/src/config.rs`:

```rust
//! Configuration types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Seconds before agent marked unhealthy
    #[serde(default = "default_health_threshold")]
    pub health_threshold_seconds: u64,

    /// Path to models config file (optional, legacy)
    pub models_config: Option<PathBuf>,

    /// Directories to scan for GGUF models
    #[serde(default)]
    pub model_dirs: Vec<PathBuf>,

    /// Path to external models config file
    pub external_models_config: Option<PathBuf>,

    /// Idle timeout in seconds before unloading models
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: u64,

    /// Path to llama-server binary
    #[serde(default = "default_llama_server_path")]
    pub llama_server_path: PathBuf,

    /// Port range for llama-server instances
    #[serde(default = "default_port_range_start")]
    pub port_range_start: u16,

    #[serde(default = "default_port_range_end")]
    pub port_range_end: u16,

    /// Reserved VRAM in MB
    #[serde(default = "default_reserve_vram_mb")]
    pub reserve_vram_mb: u64,

    /// Reserved RAM in MB
    #[serde(default = "default_reserve_ram_mb")]
    pub reserve_ram_mb: u64,
}

fn default_port() -> u16 {
    5000
}

fn default_health_threshold() -> u64 {
    120
}

fn default_idle_timeout() -> u64 {
    900 // 15 minutes
}

fn default_llama_server_path() -> PathBuf {
    PathBuf::from("llama-server")
}

fn default_port_range_start() -> u16 {
    8080
}

fn default_port_range_end() -> u16 {
    8180
}

fn default_reserve_vram_mb() -> u64 {
    500
}

fn default_reserve_ram_mb() -> u64 {
    2000
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            health_threshold_seconds: default_health_threshold(),
            models_config: None,
            model_dirs: Vec::new(),
            external_models_config: None,
            idle_timeout_seconds: default_idle_timeout(),
            llama_server_path: default_llama_server_path(),
            port_range_start: default_port_range_start(),
            port_range_end: default_port_range_end(),
            reserve_vram_mb: default_reserve_vram_mb(),
            reserve_ram_mb: default_reserve_ram_mb(),
        }
    }
}

/// Model configuration entry (legacy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
}

/// Models configuration file format (legacy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsFile {
    pub models: Vec<ModelConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orchestrator_config_defaults() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.port, 5000);
        assert_eq!(config.health_threshold_seconds, 120);
        assert_eq!(config.idle_timeout_seconds, 900);
        assert_eq!(config.port_range_start, 8080);
        assert_eq!(config.port_range_end, 8180);
    }

    #[test]
    fn test_models_file_deserialize() {
        let json = r#"{"models": [{"name": "qwen3-32b", "provider": "local"}]}"#;
        let file: ModelsFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.models.len(), 1);
        assert_eq!(file.models[0].name, "qwen3-32b");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p river-orchestrator config`
Expected: 2 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/src/config.rs
git commit -m "feat(orchestrator): extend configuration for advanced features"
```

---

### Task 15: Update main.rs with new CLI flags

**Files:**
- Modify: `crates/river-orchestrator/src/main.rs`

- [ ] **Step 1: Update CLI and initialization**

Replace `crates/river-orchestrator/src/main.rs`:

```rust
use clap::Parser;
use river_orchestrator::{
    api::create_router,
    config::{ModelsFile, OrchestratorConfig},
    discovery::ModelScanner,
    external::ExternalModelsFile,
    models::{ModelInfo, ModelProvider},
    process::ProcessConfig,
    resources::ResourceConfig,
    OrchestratorState,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "river-orchestrator")]
#[command(about = "River Engine Orchestrator - Coordination Service")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "5000")]
    port: u16,

    /// Health threshold in seconds
    #[arg(long, default_value = "120")]
    health_threshold: u64,

    /// Path to models config JSON file (legacy)
    #[arg(long)]
    models_config: Option<PathBuf>,

    /// Directories to scan for GGUF models (comma-separated)
    #[arg(long, value_delimiter = ',')]
    model_dirs: Vec<PathBuf>,

    /// Path to external models config JSON file
    #[arg(long)]
    external_models: Option<PathBuf>,

    /// Idle timeout in seconds before unloading models
    #[arg(long, default_value = "900")]
    idle_timeout: u64,

    /// Path to llama-server binary
    #[arg(long, default_value = "llama-server")]
    llama_server_path: PathBuf,

    /// Port range for llama-server instances (start-end)
    #[arg(long, default_value = "8080-8180")]
    port_range: String,

    /// Reserved VRAM in MB
    #[arg(long, default_value = "500")]
    reserve_vram_mb: u64,

    /// Reserved RAM in MB
    #[arg(long, default_value = "2000")]
    reserve_ram_mb: u64,
}

fn parse_port_range(s: &str) -> (u16, u16) {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let start = parts[0].parse().unwrap_or(8080);
        let end = parts[1].parse().unwrap_or(8180);
        (start, end)
    } else {
        (8080, 8180)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    tracing::info!("Starting River Orchestrator");
    tracing::info!("Port: {}", args.port);
    tracing::info!("Health threshold: {}s", args.health_threshold);

    let (port_start, port_end) = parse_port_range(&args.port_range);

    // Check for advanced mode (model_dirs specified)
    let use_advanced = !args.model_dirs.is_empty();

    let state = if use_advanced {
        tracing::info!("Advanced mode: scanning for GGUF models");

        // Scan for local models
        let scanner = ModelScanner::new(args.model_dirs.clone());
        let local_models = scanner.scan();
        tracing::info!("Discovered {} local models", local_models.len());

        // Load external models
        let external_models = if let Some(path) = &args.external_models {
            tracing::info!("Loading external models from {:?}", path);
            let content = std::fs::read_to_string(path)?;
            let file: ExternalModelsFile = serde_json::from_str(&content)?;
            file.external_models
        } else {
            Vec::new()
        };
        tracing::info!("Loaded {} external models", external_models.len());

        let config = OrchestratorConfig {
            port: args.port,
            health_threshold_seconds: args.health_threshold,
            models_config: args.models_config,
            model_dirs: args.model_dirs,
            external_models_config: args.external_models,
            idle_timeout_seconds: args.idle_timeout,
            llama_server_path: args.llama_server_path.clone(),
            port_range_start: port_start,
            port_range_end: port_end,
            reserve_vram_mb: args.reserve_vram_mb,
            reserve_ram_mb: args.reserve_ram_mb,
        };

        let resource_config = ResourceConfig {
            reserve_vram_bytes: args.reserve_vram_mb * 1024 * 1024,
            reserve_ram_bytes: args.reserve_ram_mb * 1024 * 1024,
        };

        let process_config = ProcessConfig {
            llama_server_path: args.llama_server_path,
            port_range_start: port_start,
            port_range_end: port_end,
            default_ctx_size: 8192,
            health_check_timeout: Duration::from_secs(5),
        };

        Arc::new(OrchestratorState::new_advanced(
            config,
            local_models,
            external_models,
            resource_config,
            process_config,
        ))
    } else {
        // Legacy mode
        let models = if let Some(path) = &args.models_config {
            tracing::info!("Loading models from {:?}", path);
            let content = std::fs::read_to_string(path)?;
            let file: ModelsFile = serde_json::from_str(&content)?;
            file.models
                .into_iter()
                .map(|m| ModelInfo::new(m.name, ModelProvider::from(m.provider.as_str())))
                .collect()
        } else {
            tracing::info!("No models config provided, starting with empty registry");
            vec![]
        };

        tracing::info!("Loaded {} models (legacy mode)", models.len());

        let config = OrchestratorConfig {
            port: args.port,
            health_threshold_seconds: args.health_threshold,
            models_config: args.models_config,
            ..Default::default()
        };

        Arc::new(OrchestratorState::new(config, models))
    };

    // Spawn background loops if in advanced mode
    if use_advanced {
        // Idle eviction loop
        let state_clone = state.clone();
        let idle_timeout = Duration::from_secs(args.idle_timeout);
        tokio::spawn(async move {
            idle_eviction_loop(state_clone, idle_timeout).await;
        });

        // Health check loop
        let process_manager = state.process_manager.clone();
        tokio::spawn(async move {
            river_orchestrator::process::health_check_loop(
                process_manager,
                Duration::from_secs(10),
            ).await;
        });
    }

    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    tracing::info!("Orchestrator listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn idle_eviction_loop(state: Arc<OrchestratorState>, timeout: Duration) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let idle_models = state.process_manager.idle_models(timeout).await;
        for model_id in idle_models {
            tracing::info!("Evicting idle model: {}", model_id);
            if let Err(e) = state.unload_model(&model_id).await {
                tracing::warn!("Failed to unload {}: {}", model_id, e);
            }
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p river-orchestrator`
Expected: Compiles successfully

- [ ] **Step 3: Run all tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/river-orchestrator/src/main.rs
git commit -m "feat(orchestrator): update CLI with advanced mode flags"
```

---

### Task 16: Update lib.rs exports

**Files:**
- Modify: `crates/river-orchestrator/src/lib.rs`

- [ ] **Step 1: Update exports**

Replace `crates/river-orchestrator/src/lib.rs`:

```rust
//! River Engine Orchestrator
//!
//! Coordination service for River Engine agents.

pub mod agents;
pub mod api;
pub mod config;
pub mod discovery;
pub mod external;
pub mod models;
pub mod process;
pub mod resources;
pub mod state;

pub use config::{ModelConfig, ModelsFile, OrchestratorConfig};
pub use state::OrchestratorState;
```

- [ ] **Step 2: Run all tests**

Run: `cargo test -p river-orchestrator`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/river-orchestrator/src/lib.rs
git commit -m "feat(orchestrator): update lib exports for advanced features"
```

---

### Task 17: Final integration test

**Files:**
- Run integration tests

- [ ] **Step 1: Build and verify**

Run: `cargo build --release -p river-orchestrator`
Expected: Build succeeds

- [ ] **Step 2: Check CLI help**

Run: `./target/release/river-orchestrator --help`
Expected: Shows all new flags including `--model-dirs`, `--external-models`, `--idle-timeout`, etc.

- [ ] **Step 3: Run all workspace tests**

Run: `cargo test`
Expected: All tests pass, record count

- [ ] **Step 4: Update STATUS.md**

Add to `docs/superpowers/STATUS.md` under Completed:

```markdown
### Plan 5: Advanced Orchestrator ✅
- Model discovery via GGUF header parsing
- GPU/VRAM and CPU memory tracking with swap detection
- llama-server process lifecycle management
- LiteLLM integration for external models
- On-demand model loading with idle eviction
- New API endpoints: `/model/request`, `/model/release`, enhanced `/models/available`
- XX tests passing (update with actual count)
- Binary: `river-orchestrator --model-dirs /models --external-models config.json`
```

- [ ] **Step 5: Commit final changes**

```bash
git add docs/superpowers/STATUS.md
git commit -m "docs: update STATUS.md with Plan 5 completion"
```

---

## Summary

This plan implements the advanced orchestrator in 18 tasks across 5 chunks:

1. **Chunk 1 (Tasks 1-4)**: GGUF parsing and model discovery
2. **Chunk 2 (Tasks 5-8)**: Resource management (GPU, memory, tracker)
3. **Chunk 3 (Tasks 9-10b)**: Process lifecycle (ports, manager, health monitoring)
4. **Chunk 4 (Tasks 11-13)**: Extended state with eviction, API endpoints
5. **Chunk 5 (Tasks 14-17)**: CLI updates and integration

Each task follows TDD with tests before implementation, and commits after each logical unit.

**Key features implemented:**
- GGUF header parsing with VRAM estimation
- GPU discovery via nvidia-smi
- System memory tracking with swap detection
- Port allocation for llama-server instances
- Process spawning, health monitoring, and cleanup
- On-demand eviction for releasable models
- Background health check loop (10s interval)
- Idle timeout eviction loop (configurable)
- External model support via LiteLLM
