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
            reserve_vram_bytes: 500 * 1024 * 1024,     // 500MB
            reserve_ram_bytes: 2 * 1024 * 1024 * 1024, // 2GB
        }
    }
}

/// Central tracker for all device resources
pub struct ResourceTracker {
    devices: RwLock<Vec<DeviceResources>>,
    /// Config is applied during construction; kept for potential runtime reconfiguration
    #[allow(dead_code)]
    config: ResourceConfig,
}

impl ResourceTracker {
    /// Initialize tracker by detecting available devices
    pub fn new(config: ResourceConfig) -> Self {
        let mut devices = Vec::new();

        // Detect GPUs and create DeviceResources for each
        let gpus = detect_gpus();
        for gpu in gpus {
            tracing::info!(
                "Detected GPU {}: {} with {} bytes VRAM",
                gpu.id,
                gpu.name,
                gpu.total_vram
            );
            devices.push(DeviceResources::new(
                DeviceId::Gpu(gpu.id),
                gpu.total_vram,
                config.reserve_vram_bytes,
            ));
        }

        // Add CPU as fallback
        let system_memory = SystemMemory::current();
        tracing::info!("CPU available with {} bytes RAM", system_memory.total_ram);
        devices.push(DeviceResources::new(
            DeviceId::Cpu,
            system_memory.total_ram,
            config.reserve_ram_bytes,
        ));

        tracing::info!(
            "ResourceTracker initialized with {} device(s)",
            devices.len()
        );

        Self {
            devices: RwLock::new(devices),
            config,
        }
    }

    /// Find a device that can fit the required memory (GPUs first, then CPU)
    pub async fn find_device_for(&self, bytes_needed: u64) -> Option<DeviceId> {
        let devices = self.devices.read().await;

        // Try GPUs first
        for device in devices.iter() {
            if matches!(device.device, DeviceId::Gpu(_)) && device.can_fit(bytes_needed) {
                return Some(device.device);
            }
        }

        // Fall back to CPU
        for device in devices.iter() {
            if matches!(device.device, DeviceId::Cpu) && device.can_fit(bytes_needed) {
                return Some(device.device);
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

        for device in devices.iter() {
            if device.device == DeviceId::Cpu {
                return device.allocated;
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
    fn from(resources: &DeviceResources) -> Self {
        Self {
            device: resources.device,
            total_memory: resources.total_memory,
            allocated: resources.allocated,
            available: resources.available(),
            allocations: resources
                .allocations
                .iter()
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
        let resources = tracker.get_all_resources().await;
        assert!(!resources.is_empty()); // At least CPU
    }

    #[tokio::test]
    async fn test_resource_allocation() {
        let tracker = ResourceTracker::new(ResourceConfig::default());
        let allocated = tracker
            .allocate("test-model", DeviceId::Cpu, 1_000_000_000)
            .await;
        assert!(allocated);
        assert_eq!(tracker.cpu_allocated().await, 1_000_000_000);
        tracker.release("test-model", DeviceId::Cpu).await;
        assert_eq!(tracker.cpu_allocated().await, 0);
    }

    #[tokio::test]
    async fn test_find_device_for() {
        let tracker = ResourceTracker::new(ResourceConfig::default());

        // Should be able to find a device for a small allocation
        let device = tracker.find_device_for(1_000_000).await;
        assert!(device.is_some());
    }

    #[tokio::test]
    async fn test_allocation_limits() {
        let tracker = ResourceTracker::new(ResourceConfig::default());

        // Get available memory on CPU
        let resources = tracker.get_all_resources().await;
        let cpu_resource = resources
            .iter()
            .find(|r| r.device == DeviceId::Cpu)
            .unwrap();
        let available = cpu_resource.available;

        // Try to allocate more than available
        let allocated = tracker
            .allocate("too-large", DeviceId::Cpu, available + 1)
            .await;
        assert!(!allocated);

        // Should still have 0 allocated
        assert_eq!(tracker.cpu_allocated().await, 0);
    }

    #[tokio::test]
    async fn test_multiple_allocations() {
        let tracker = ResourceTracker::new(ResourceConfig::default());

        // Allocate multiple models
        assert!(
            tracker
                .allocate("model1", DeviceId::Cpu, 1_000_000_000)
                .await
        );
        assert!(tracker.allocate("model2", DeviceId::Cpu, 500_000_000).await);

        assert_eq!(tracker.cpu_allocated().await, 1_500_000_000);

        // Release one
        tracker.release("model1", DeviceId::Cpu).await;
        assert_eq!(tracker.cpu_allocated().await, 500_000_000);

        // Release the other
        tracker.release("model2", DeviceId::Cpu).await;
        assert_eq!(tracker.cpu_allocated().await, 0);
    }

    #[tokio::test]
    async fn test_snapshot_conversion() {
        let tracker = ResourceTracker::new(ResourceConfig::default());

        // Allocate something
        tracker
            .allocate("test-model", DeviceId::Cpu, 1_000_000_000)
            .await;

        // Get snapshot
        let snapshots = tracker.get_all_resources().await;
        let cpu_snapshot = snapshots
            .iter()
            .find(|s| s.device == DeviceId::Cpu)
            .unwrap();

        assert_eq!(cpu_snapshot.allocated, 1_000_000_000);
        assert_eq!(cpu_snapshot.allocations.len(), 1);
        assert_eq!(cpu_snapshot.allocations[0].0, "test-model");
        assert_eq!(cpu_snapshot.allocations[0].1, 1_000_000_000);
    }
}
