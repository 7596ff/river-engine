//! GPU discovery

use std::process::Command;

/// Information about a detected GPU
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub id: u32,
    pub name: String,
    pub total_vram: u64,  // bytes
}

/// Discover available GPUs
pub fn detect_gpus() -> Vec<GpuInfo> {
    // Try NVIDIA first
    if let Some(gpus) = detect_nvidia_gpus() {
        if !gpus.is_empty() {
            return gpus;
        }
    }
    // Return empty if no GPUs
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
        // Just ensure detection doesn't panic
        let _gpus = detect_gpus();
    }
}
