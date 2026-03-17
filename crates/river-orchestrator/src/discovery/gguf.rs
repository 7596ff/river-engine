//! GGUF model metadata and resource estimation

use std::fmt;

/// Quantization types supported in GGUF models
#[derive(Debug, Clone, PartialEq, Eq)]
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
}
