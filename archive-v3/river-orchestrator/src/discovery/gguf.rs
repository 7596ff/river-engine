//! GGUF model metadata and resource estimation

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use river_core::RiverError;

/// Quantization types supported in GGUF models
/// Names match GGUF specification naming convention
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
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
    /// Parse quantization type from string (case-insensitive)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "q4_0" => QuantizationType::Q4_0,
            "q4_1" => QuantizationType::Q4_1,
            "q4_k_m" => QuantizationType::Q4_K_M,
            "q4_k_s" => QuantizationType::Q4_K_S,
            "q5_0" => QuantizationType::Q5_0,
            "q5_1" => QuantizationType::Q5_1,
            "q5_k_m" => QuantizationType::Q5_K_M,
            "q5_k_s" => QuantizationType::Q5_K_S,
            "q6_k" => QuantizationType::Q6_K,
            "q8_0" => QuantizationType::Q8_0,
            "f16" => QuantizationType::F16,
            "f32" => QuantizationType::F32,
            _ => QuantizationType::Unknown(s.to_string()),
        }
    }
}

impl fmt::Display for QuantizationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantizationType::Q4_0 => write!(f, "Q4_0"),
            QuantizationType::Q4_1 => write!(f, "Q4_1"),
            QuantizationType::Q4_K_M => write!(f, "Q4_K_M"),
            QuantizationType::Q4_K_S => write!(f, "Q4_K_S"),
            QuantizationType::Q5_0 => write!(f, "Q5_0"),
            QuantizationType::Q5_1 => write!(f, "Q5_1"),
            QuantizationType::Q5_K_M => write!(f, "Q5_K_M"),
            QuantizationType::Q5_K_S => write!(f, "Q5_K_S"),
            QuantizationType::Q6_K => write!(f, "Q6_K"),
            QuantizationType::Q8_0 => write!(f, "Q8_0"),
            QuantizationType::F16 => write!(f, "F16"),
            QuantizationType::F32 => write!(f, "F32"),
            QuantizationType::Unknown(s) => write!(f, "Unknown({})", s),
        }
    }
}

/// Metadata extracted from a GGUF model file
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
    /// Estimate KV cache memory requirements in bytes
    fn estimate_kv_cache(&self) -> u64 {
        // KV cache = layers * hidden_dim * 4 * context_length
        self.layers as u64 * self.hidden_dim as u64 * 4 * self.context_length as u64
    }

    /// Estimate total VRAM requirements in bytes
    pub fn estimate_vram(&self) -> u64 {
        // Total VRAM = file_size + kv_cache + 500MB overhead
        let overhead = 500 * 1024 * 1024; // 500 MB in bytes
        self.file_size + self.estimate_kv_cache() + overhead
    }
}

/// GGUF magic number constant
const GGUF_MAGIC: u32 = 0x46554747;

/// GGUF value types for metadata key-value pairs
/// All variants defined per GGUF spec; not all are currently parsed
#[derive(Debug, Clone)]
#[allow(dead_code)]
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

/// Read a u32 value from the reader (little-endian)
fn read_u32<R: Read>(reader: &mut R) -> Result<u32, RiverError> {
    let mut buf = [0u8; 4];
    reader
        .read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read u32: {}", e)))?;
    Ok(u32::from_le_bytes(buf))
}

/// Read a u64 value from the reader (little-endian)
fn read_u64<R: Read>(reader: &mut R) -> Result<u64, RiverError> {
    let mut buf = [0u8; 8];
    reader
        .read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read u64: {}", e)))?;
    Ok(u64::from_le_bytes(buf))
}

/// Read an i32 value from the reader (little-endian)
fn read_i32<R: Read>(reader: &mut R) -> Result<i32, RiverError> {
    let mut buf = [0u8; 4];
    reader
        .read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read i32: {}", e)))?;
    Ok(i32::from_le_bytes(buf))
}

/// Maximum allowed string length in GGUF metadata (16 MB)
const MAX_STRING_LENGTH: u64 = 16 * 1024 * 1024;

/// Maximum number of metadata key-value pairs (sanity check)
const MAX_METADATA_COUNT: u64 = 100_000;

/// Read a string from the reader (length-prefixed)
fn read_string<R: Read>(reader: &mut R) -> Result<String, RiverError> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_LENGTH {
        return Err(RiverError::orchestrator(format!(
            "String length {} exceeds maximum allowed ({})",
            len, MAX_STRING_LENGTH
        )));
    }
    let mut buf = vec![0u8; len as usize];
    reader
        .read_exact(&mut buf)
        .map_err(|e| RiverError::orchestrator(format!("Failed to read string: {}", e)))?;
    String::from_utf8(buf)
        .map_err(|e| RiverError::orchestrator(format!("Invalid UTF-8 in string: {}", e)))
}

/// Read a GGUF value based on its type ID
fn read_gguf_value<R: Read>(reader: &mut R, type_id: u32) -> Result<GgufValue, RiverError> {
    match type_id {
        0 => Ok(GgufValue::U32(read_u32(reader)?)),
        1 => Ok(GgufValue::I32(read_i32(reader)?)),
        4 => Ok(GgufValue::U64(read_u64(reader)?)),
        8 => {
            let val = read_string(reader)?;
            Ok(GgufValue::String(val))
        }
        _ => Err(RiverError::orchestrator(format!(
            "Unsupported GGUF value type: {}",
            type_id
        ))),
    }
}

/// Read a metadata key-value pair
fn read_metadata_kv<R: Read>(reader: &mut R) -> Result<(String, GgufValue), RiverError> {
    let key = read_string(reader)?;
    let value_type = read_u32(reader)?;
    let value = read_gguf_value(reader, value_type)?;
    Ok((key, value))
}

/// Extract quantization type from filename
fn extract_quantization_from_filename(path: &PathBuf) -> QuantizationType {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Common GGUF quantization patterns in filenames
    let patterns = [
        ("Q4_K_M", QuantizationType::Q4_K_M),
        ("Q4_K_S", QuantizationType::Q4_K_S),
        ("Q5_K_M", QuantizationType::Q5_K_M),
        ("Q5_K_S", QuantizationType::Q5_K_S),
        ("Q6_K", QuantizationType::Q6_K),
        ("Q8_0", QuantizationType::Q8_0),
        ("Q4_0", QuantizationType::Q4_0),
        ("Q4_1", QuantizationType::Q4_1),
        ("Q5_0", QuantizationType::Q5_0),
        ("Q5_1", QuantizationType::Q5_1),
        ("F16", QuantizationType::F16),
        ("F32", QuantizationType::F32),
    ];

    for (pattern, quant_type) in patterns.iter() {
        if filename.to_uppercase().contains(pattern) {
            return quant_type.clone();
        }
    }

    QuantizationType::Unknown("unknown".to_string())
}

/// Estimate model parameters based on file size and quantization
fn estimate_parameters(file_size: u64, quantization: &QuantizationType) -> u64 {
    // Approximate bytes per parameter for different quantizations
    let bytes_per_param = match quantization {
        QuantizationType::Q4_0 | QuantizationType::Q4_1 => 0.5,
        QuantizationType::Q4_K_M | QuantizationType::Q4_K_S => 0.5,
        QuantizationType::Q5_0 | QuantizationType::Q5_1 => 0.625,
        QuantizationType::Q5_K_M | QuantizationType::Q5_K_S => 0.625,
        QuantizationType::Q6_K => 0.75,
        QuantizationType::Q8_0 => 1.0,
        QuantizationType::F16 => 2.0,
        QuantizationType::F32 => 4.0,
        QuantizationType::Unknown(_) => 0.5, // Default to Q4 estimate
    };

    (file_size as f64 / bytes_per_param) as u64
}

/// Parse GGUF file and extract metadata
pub fn parse_gguf(path: &PathBuf) -> Result<GgufMetadata, RiverError> {
    let file = File::open(path)
        .map_err(|e| RiverError::orchestrator(format!("Failed to open GGUF file: {}", e)))?;

    let file_size = file
        .metadata()
        .map_err(|e| RiverError::orchestrator(format!("Failed to read file metadata: {}", e)))?
        .len();

    let mut reader = BufReader::new(file);

    // Read and verify magic number
    let magic = read_u32(&mut reader)?;
    if magic != GGUF_MAGIC {
        return Err(RiverError::orchestrator(format!(
            "Invalid GGUF magic number: expected 0x{:X}, got 0x{:X}",
            GGUF_MAGIC, magic
        )));
    }

    // Read version
    let version = read_u32(&mut reader)?;
    if version != 2 && version != 3 {
        return Err(RiverError::orchestrator(format!(
            "Unsupported GGUF version: {}",
            version
        )));
    }

    // Read tensor count and metadata count
    let _tensor_count = read_u64(&mut reader)?;
    let metadata_kv_count = read_u64(&mut reader)?;

    if metadata_kv_count > MAX_METADATA_COUNT {
        return Err(RiverError::orchestrator(format!(
            "Metadata count {} exceeds maximum allowed ({})",
            metadata_kv_count, MAX_METADATA_COUNT
        )));
    }

    // Parse metadata key-value pairs
    let mut name = String::from("unknown");
    let mut architecture = String::from("unknown");
    let mut context_length = 2048u32;
    let mut layers = 32u32;
    let mut hidden_dim = 4096u32;

    for _ in 0..metadata_kv_count {
        let (key, value) = read_metadata_kv(&mut reader)?;

        match key.as_str() {
            "general.name" => {
                if let GgufValue::String(s) = value {
                    name = s;
                }
            }
            "general.architecture" => {
                if let GgufValue::String(s) = value {
                    architecture = s;
                }
            }
            k if k.ends_with(".context_length") => {
                if let GgufValue::U32(v) = value {
                    context_length = v;
                } else if let GgufValue::U64(v) = value {
                    context_length = v as u32;
                }
            }
            k if k.ends_with(".block_count") => {
                if let GgufValue::U32(v) = value {
                    layers = v;
                } else if let GgufValue::U64(v) = value {
                    layers = v as u32;
                }
            }
            k if k.ends_with(".embedding_length") => {
                if let GgufValue::U32(v) = value {
                    hidden_dim = v;
                } else if let GgufValue::U64(v) = value {
                    hidden_dim = v as u32;
                }
            }
            _ => {}
        }
    }

    // Extract quantization from filename
    let quantization = extract_quantization_from_filename(path);

    // Estimate parameters
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantization_type_from_str_lowercase() {
        assert_eq!(QuantizationType::from_str("q4_0"), QuantizationType::Q4_0);
        assert_eq!(QuantizationType::from_str("q4_1"), QuantizationType::Q4_1);
        assert_eq!(QuantizationType::from_str("q4_k_m"), QuantizationType::Q4_K_M);
        assert_eq!(QuantizationType::from_str("q4_k_s"), QuantizationType::Q4_K_S);
        assert_eq!(QuantizationType::from_str("q5_0"), QuantizationType::Q5_0);
        assert_eq!(QuantizationType::from_str("q5_1"), QuantizationType::Q5_1);
        assert_eq!(QuantizationType::from_str("q5_k_m"), QuantizationType::Q5_K_M);
        assert_eq!(QuantizationType::from_str("q5_k_s"), QuantizationType::Q5_K_S);
        assert_eq!(QuantizationType::from_str("q6_k"), QuantizationType::Q6_K);
        assert_eq!(QuantizationType::from_str("q8_0"), QuantizationType::Q8_0);
        assert_eq!(QuantizationType::from_str("f16"), QuantizationType::F16);
        assert_eq!(QuantizationType::from_str("f32"), QuantizationType::F32);
    }

    #[test]
    fn test_quantization_type_from_str_uppercase() {
        assert_eq!(QuantizationType::from_str("Q4_0"), QuantizationType::Q4_0);
        assert_eq!(QuantizationType::from_str("Q5_K_M"), QuantizationType::Q5_K_M);
        assert_eq!(QuantizationType::from_str("F16"), QuantizationType::F16);
    }

    #[test]
    fn test_quantization_type_from_str_mixed_case() {
        assert_eq!(QuantizationType::from_str("Q4_k_M"), QuantizationType::Q4_K_M);
        assert_eq!(QuantizationType::from_str("q5_K_s"), QuantizationType::Q5_K_S);
    }

    #[test]
    fn test_quantization_type_from_str_unknown() {
        match QuantizationType::from_str("q3_k") {
            QuantizationType::Unknown(s) => assert_eq!(s, "q3_k"),
            _ => panic!("Expected Unknown variant"),
        }
    }

    #[test]
    fn test_vram_estimation_small_model() {
        let metadata = GgufMetadata {
            name: "test-model".to_string(),
            architecture: "llama".to_string(),
            parameters: 7_000_000_000,
            quantization: QuantizationType::Q4_0,
            context_length: 2048,
            layers: 32,
            hidden_dim: 4096,
            file_size: 4_000_000_000, // 4 GB
        };

        let vram = metadata.estimate_vram();

        // Expected: file_size (4GB) + kv_cache + 500MB overhead
        // kv_cache = 32 * 4096 * 4 * 2048 = 1,073,741,824 bytes (1 GB)
        // total = 4GB + 1GB + 500MB = 5.5GB = 5,905,580,032 bytes
        let expected_kv_cache = 32u64 * 4096 * 4 * 2048;
        let expected_vram = 4_000_000_000 + expected_kv_cache + 500 * 1024 * 1024;

        assert_eq!(vram, expected_vram);
    }

    #[test]
    fn test_vram_estimation_large_context() {
        let metadata = GgufMetadata {
            name: "test-model-large-context".to_string(),
            architecture: "llama".to_string(),
            parameters: 13_000_000_000,
            quantization: QuantizationType::Q5_K_M,
            context_length: 8192,
            layers: 40,
            hidden_dim: 5120,
            file_size: 8_000_000_000, // 8 GB
        };

        let vram = metadata.estimate_vram();

        // kv_cache = 40 * 5120 * 4 * 8192
        let expected_kv_cache = 40u64 * 5120 * 4 * 8192;
        let expected_vram = 8_000_000_000 + expected_kv_cache + 500 * 1024 * 1024;

        assert_eq!(vram, expected_vram);
        // Verify KV cache is substantial for large context
        assert!(expected_kv_cache > 6_000_000_000); // > 6GB
    }

    #[test]
    fn test_estimate_kv_cache_direct() {
        let metadata = GgufMetadata {
            name: "test".to_string(),
            architecture: "llama".to_string(),
            parameters: 7_000_000_000,
            quantization: QuantizationType::Q4_0,
            context_length: 2048,
            layers: 32,
            hidden_dim: 4096,
            file_size: 4_000_000_000,
        };

        let kv_cache = metadata.estimate_kv_cache();
        let expected = 32u64 * 4096 * 4 * 2048;
        assert_eq!(kv_cache, expected);
    }

    #[test]
    fn test_extract_quantization_from_filename() {
        let path_q4_k_m = PathBuf::from("/models/llama-7b-Q4_K_M.gguf");
        assert_eq!(
            extract_quantization_from_filename(&path_q4_k_m),
            QuantizationType::Q4_K_M
        );

        let path_q8_0 = PathBuf::from("/models/mistral-Q8_0.gguf");
        assert_eq!(
            extract_quantization_from_filename(&path_q8_0),
            QuantizationType::Q8_0
        );

        let path_lowercase = PathBuf::from("/models/model-q4_k_m.gguf");
        assert_eq!(
            extract_quantization_from_filename(&path_lowercase),
            QuantizationType::Q4_K_M
        );

        let path_unknown = PathBuf::from("/models/model.gguf");
        match extract_quantization_from_filename(&path_unknown) {
            QuantizationType::Unknown(_) => {}
            _ => panic!("Expected Unknown quantization type"),
        }
    }

    #[test]
    fn test_estimate_parameters() {
        // 4GB Q4_K_M file should be approximately 8B parameters
        // 4GB / 0.5 bytes per param = 8B params
        let file_size = 4_000_000_000u64; // 4GB
        let quantization = QuantizationType::Q4_K_M;
        let params = estimate_parameters(file_size, &quantization);

        assert_eq!(params, 8_000_000_000); // 8B parameters

        // Test Q8_0 (1 byte per parameter)
        let params_q8 = estimate_parameters(file_size, &QuantizationType::Q8_0);
        assert_eq!(params_q8, 4_000_000_000); // 4B parameters

        // Test F16 (2 bytes per parameter)
        let params_f16 = estimate_parameters(file_size, &QuantizationType::F16);
        assert_eq!(params_f16, 2_000_000_000); // 2B parameters
    }
}
