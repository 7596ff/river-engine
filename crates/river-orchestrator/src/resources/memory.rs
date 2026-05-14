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

        let mut total_ram = None;
        let mut available_ram = None;
        let mut total_swap = None;
        let mut available_swap = None;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let key = parts[0];
            let value = parts[1].parse::<u64>().ok()?;

            // Values in /proc/meminfo are in kB, convert to bytes
            let value_bytes = value * 1024;

            match key {
                "MemTotal:" => total_ram = Some(value_bytes),
                "MemAvailable:" => available_ram = Some(value_bytes),
                "SwapTotal:" => total_swap = Some(value_bytes),
                "SwapFree:" => available_swap = Some(value_bytes),
                _ => {}
            }
        }

        Some(Self {
            total_ram: total_ram?,
            available_ram: available_ram?,
            total_swap: total_swap.unwrap_or(0),
            available_swap: available_swap.unwrap_or(0),
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
        // On Linux should have positive values
        #[cfg(target_os = "linux")]
        {
            assert!(mem.total_ram > 0);
        }
    }

    #[test]
    fn test_swap_detection() {
        let mem = SystemMemory {
            total_ram: 64_000_000_000,
            available_ram: 32_000_000_000,
            total_swap: 32_000_000_000,
            available_swap: 32_000_000_000,
        };
        // 20GB model + 10GB used = 30GB, fits in 32GB available
        assert!(!mem.would_use_swap(20_000_000_000, 10_000_000_000));
        // 30GB model + 10GB used = 40GB, needs 8GB swap
        assert!(mem.would_use_swap(30_000_000_000, 10_000_000_000));
        assert_eq!(
            mem.estimated_swap_usage(30_000_000_000, 10_000_000_000),
            8_000_000_000
        );
    }
}
