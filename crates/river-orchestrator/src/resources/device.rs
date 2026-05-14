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
    pub fn to_api_string(&self) -> String {
        match self {
            DeviceId::Gpu(idx) => format!("gpu:{}", idx),
            DeviceId::Cpu => "cpu".to_string(),
        }
    }

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
    pub allocations: HashMap<String, u64>, // model_id -> bytes
}

impl DeviceResources {
    pub fn new(device: DeviceId, total_memory: u64, reserved: u64) -> Self {
        Self {
            device,
            total_memory,
            reserved,
            allocated: 0,
            allocations: HashMap::new(),
        }
    }

    pub fn available(&self) -> u64 {
        self.total_memory
            .saturating_sub(self.reserved)
            .saturating_sub(self.allocated)
    }

    pub fn can_fit(&self, bytes: u64) -> bool {
        self.available() >= bytes
    }

    pub fn allocate(&mut self, model_id: &str, bytes: u64) -> bool {
        if !self.can_fit(bytes) {
            return false;
        }

        self.allocations.insert(model_id.to_string(), bytes);
        self.allocated += bytes;
        true
    }

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
        assert_eq!(DeviceId::Cpu.to_api_string(), "cpu");
        assert_eq!(DeviceId::Gpu(0).to_api_string(), "gpu:0");
        assert_eq!(DeviceId::Gpu(42).to_api_string(), "gpu:42");
    }

    #[test]
    fn test_device_id_parsing() {
        assert_eq!(DeviceId::from_api_string("cpu"), Some(DeviceId::Cpu));
        assert_eq!(DeviceId::from_api_string("gpu:0"), Some(DeviceId::Gpu(0)));
        assert_eq!(DeviceId::from_api_string("gpu:42"), Some(DeviceId::Gpu(42)));
        assert_eq!(DeviceId::from_api_string("invalid"), None);
        assert_eq!(DeviceId::from_api_string("gpu:"), None);
        assert_eq!(DeviceId::from_api_string("gpu:abc"), None);
    }

    #[test]
    fn test_device_resources_allocation() {
        let mut resources = DeviceResources::new(DeviceId::Gpu(0), 1000, 100);

        // Check initial state
        assert_eq!(resources.available(), 900);
        assert!(resources.can_fit(500));
        assert!(resources.can_fit(900));
        assert!(!resources.can_fit(901));

        // Allocate memory
        assert!(resources.allocate("model1", 400));
        assert_eq!(resources.available(), 500);
        assert_eq!(resources.allocated, 400);

        // Allocate more
        assert!(resources.allocate("model2", 300));
        assert_eq!(resources.available(), 200);
        assert_eq!(resources.allocated, 700);

        // Try to allocate too much
        assert!(!resources.allocate("model3", 300));
        assert_eq!(resources.available(), 200);
        assert_eq!(resources.allocated, 700);

        // Release memory
        resources.release("model1");
        assert_eq!(resources.available(), 600);
        assert_eq!(resources.allocated, 300);

        // Release again (should be idempotent)
        resources.release("model1");
        assert_eq!(resources.available(), 600);
        assert_eq!(resources.allocated, 300);

        // Release second model
        resources.release("model2");
        assert_eq!(resources.available(), 900);
        assert_eq!(resources.allocated, 0);
    }
}
