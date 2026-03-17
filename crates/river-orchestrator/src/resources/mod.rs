pub mod device;
pub mod gpu;

pub use device::{DeviceId, DeviceResources};
pub use gpu::{detect_gpus, GpuInfo};
