# Advanced Orchestrator Design Specification

**Version:** 1.1
**Date:** 2026-03-16
**Status:** Draft
**Extends:** `2026-03-16-orchestrator-minimal-design.md`

---

## 1. Overview

This specification extends the minimal orchestrator with:

1. **Model Discovery** - Scan local GGUF files, parse headers for metadata
2. **Resource Management** - GPU/VRAM tracking, CPU memory support
3. **Process Lifecycle** - Spawn/monitor/evict llama-server instances
4. **LiteLLM Integration** - Route requests to external API models

### Design Principles

- **On-demand loading**: Models spin up when requested, not at startup
- **Resource-aware**: Track VRAM/RAM, refuse loads that won't fit
- **Graceful eviction**: Idle timeout + priority-based eviction
- **Mixed backends**: Local GGUF models + external APIs coexist

---

## 2. Architecture

### 2.1 Component Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Orchestrator                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Model     │  │  Resource   │  │    Process Manager      │  │
│  │  Registry   │  │   Tracker   │  │  (llama-server spawner) │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│  ┌─────────────┐  ┌─────────────┐                               │
│  │   Agent     │  │   LiteLLM   │                               │
│  │  Registry   │  │   Router    │                               │
│  └─────────────┘  └─────────────┘                               │
└─────────────────────────────────────────────────────────────────┘
        ▲                   │
        │ heartbeat         │ spawn/kill
        │                   ▼
┌───────────────┐    ┌─────────────────┐
│   Gateways    │    │  llama-server   │
│ (thomas, etc) │    │   instances     │
└───────────────┘    └─────────────────┘
```

### 2.2 New Modules

```
crates/river-orchestrator/src/
├── discovery/
│   ├── mod.rs           # Model discovery exports
│   ├── scanner.rs       # Directory scanning
│   └── gguf.rs          # GGUF header parsing
├── resources/
│   ├── mod.rs           # Resource tracking exports
│   ├── gpu.rs           # GPU discovery and VRAM tracking
│   └── memory.rs        # System memory tracking
├── process/
│   ├── mod.rs           # Process management exports
│   ├── manager.rs       # Spawn/monitor/kill processes
│   └── llama_server.rs  # llama-server specific logic
└── litellm/
    └── mod.rs           # LiteLLM routing
```

---

## 3. Model Discovery

### 3.1 Directory Scanning

Scan configured model directories for GGUF files:

```rust
pub struct ModelScanner {
    model_dirs: Vec<PathBuf>,
}

impl ModelScanner {
    pub fn scan(&self) -> Vec<LocalModel> {
        // Walk each directory
        // Find *.gguf files
        // Parse headers
        // Handle ID collisions (see 3.5)
        // Return model list
    }
}
```

**Configuration:**
```json
{
  "model_dirs": ["/models", "/home/user/.cache/models"]
}
```

### 3.2 GGUF Header Parsing

GGUF files contain metadata in their headers. Parse to extract:

- Model name/architecture
- Parameter count
- Quantization type (Q4_K_M, Q8_0, F16, etc.)
- Context length
- Layer count and hidden dimension (for VRAM calculation)
- Tensor information for VRAM calculation

```rust
pub struct GgufMetadata {
    pub name: String,
    pub architecture: String,
    pub parameters: u64,
    pub quantization: QuantizationType,
    pub context_length: u32,
    pub layers: u32,              // From "llama.block_count" or equivalent
    pub hidden_dim: u32,          // From "llama.embedding_length" or equivalent
    pub file_size: u64,
    pub estimated_vram: u64,      // bytes, computed from above fields
}

pub enum QuantizationType {
    Q4_0, Q4_1, Q4_K_M, Q4_K_S,
    Q5_0, Q5_1, Q5_K_M, Q5_K_S,
    Q6_K, Q8_0,
    F16, F32,
    Unknown(String),
}
```

**GGUF Header Keys:**
- `llama.block_count` → `layers`
- `llama.embedding_length` → `hidden_dim`
- `general.architecture` → `architecture`
- `general.name` → `name`
- `llama.context_length` → `context_length`

Different architectures use different key prefixes (llama, qwen2, etc.). The parser should handle common architectures.

### 3.3 VRAM Estimation

Calculate VRAM from GGUF metadata:

```rust
impl GgufMetadata {
    pub fn estimate_vram(&self) -> u64 {
        // Base: file size (weights are largest component)
        // Add: KV cache estimate based on context length
        // Add: ~500MB overhead for llama-server

        let kv_cache = self.estimate_kv_cache();
        let overhead = 500 * 1024 * 1024; // 500MB

        self.file_size + kv_cache + overhead
    }

    fn estimate_kv_cache(&self) -> u64 {
        // KV cache size depends on:
        // - context_length
        // - number of layers
        // - hidden dimension
        // Formula: 2 bytes per token per layer * hidden_dim * 2 (K+V)
        // For 32k context, 32 layers, 4096 hidden: ~4GB

        let bytes_per_token = (self.layers as u64) * (self.hidden_dim as u64) * 4;
        (self.context_length as u64) * bytes_per_token
    }
}
```

### 3.4 Local Model Registry

```rust
pub struct LocalModel {
    pub id: String,              // Derived from filename (see 3.5 for collisions)
    pub path: PathBuf,
    pub metadata: GgufMetadata,
    pub status: ModelStatus,
}

/// Internal status tracking using Instant for performance.
/// API responses compute idle_seconds dynamically from last_request.
pub enum ModelStatus {
    Available,                   // Can be loaded
    Loading,                     // Currently spinning up
    Loaded {
        pid: u32,
        port: u16,
        device: DeviceId,
        loaded_at: Instant,      // Internal only, not serialized
        last_request: Instant,   // Internal only, compute idle_seconds for API
    },
    Unloading,                   // Shutting down
    Error(String),               // Failed to load
}

impl ModelStatus {
    /// Compute idle seconds for API responses
    pub fn idle_seconds(&self) -> Option<u64> {
        match self {
            ModelStatus::Loaded { last_request, .. } => {
                Some(last_request.elapsed().as_secs())
            }
            _ => None,
        }
    }
}
```

### 3.5 Model ID Collision Handling

When multiple directories contain files with the same name:

1. First scanned directory wins (model_dirs order matters)
2. Subsequent duplicates are skipped with a warning log
3. Model ID is the filename stem (e.g., `llama3-8b-q4_k_m.gguf` → `llama3-8b-q4_k_m`)

```rust
impl ModelScanner {
    pub fn scan(&self) -> Vec<LocalModel> {
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut models = Vec::new();

        for dir in &self.model_dirs {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.extension() == Some("gguf") {
                    let id = path.file_stem().to_string();
                    if seen_ids.contains(&id) {
                        tracing::warn!(
                            "Skipping duplicate model ID '{}' at {}",
                            id, path.display()
                        );
                        continue;
                    }
                    seen_ids.insert(id.clone());
                    // Parse and add model...
                }
            }
        }
        models
    }
}
```

---

## 4. Resource Management

### 4.1 GPU Discovery

Detect available GPUs and their VRAM:

```rust
pub struct GpuInfo {
    pub id: u32,
    pub name: String,
    pub total_vram: u64,       // bytes
    pub used_vram: u64,        // bytes (by our processes)
    pub device_path: String,   // e.g., "/dev/nvidia0"
}

pub struct GpuDiscovery;

impl GpuDiscovery {
    pub fn detect() -> Vec<GpuInfo> {
        // Try nvidia-smi first (NVIDIA GPUs)
        // Fall back to sysfs for AMD/Intel
        // Return empty vec if no GPUs
    }
}
```

**NVIDIA detection:**
```bash
nvidia-smi --query-gpu=index,name,memory.total --format=csv,noheader,nounits
```

**AMD detection (sysfs):**
```
/sys/class/drm/card*/device/mem_info_vram_total
```

### 4.2 Memory Tracking

Track system RAM and swap for CPU-only inference:

```rust
pub struct SystemMemory {
    pub total_ram: u64,
    pub available_ram: u64,
    pub used_by_models: u64,
    pub total_swap: u64,
    pub available_swap: u64,
}

impl SystemMemory {
    pub fn current() -> Self {
        // Read from /proc/meminfo:
        // - MemTotal, MemAvailable
        // - SwapTotal, SwapFree
    }

    /// Check if loading a model would require swap
    pub fn would_use_swap(&self, model_bytes: u64) -> bool {
        let after_load = self.used_by_models + model_bytes;
        after_load > self.available_ram
    }

    /// Estimate how much swap would be used
    pub fn estimated_swap_usage(&self, model_bytes: u64) -> u64 {
        let after_load = self.used_by_models + model_bytes;
        after_load.saturating_sub(self.available_ram)
    }
}
```

**Swap Warning:**
When loading a model on CPU that would require swap, log a warning but proceed:

```rust
if system_memory.would_use_swap(model.metadata.estimated_vram) {
    let swap_needed = system_memory.estimated_swap_usage(model.metadata.estimated_vram);
    tracing::warn!(
        "Model '{}' will use ~{:.1}GB swap. Expect slow inference due to memory pressure.",
        model.id,
        swap_needed as f64 / 1_073_741_824.0
    );
}
```

### 4.3 Device Abstraction

Unified interface for GPU and CPU:

```rust
/// Device identifier with consistent serialization
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceId {
    Gpu(u32),      // GPU index
    Cpu,           // CPU inference
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

pub struct DeviceResources {
    pub device: DeviceId,
    pub total_memory: u64,
    pub reserved: u64,                          // From config (reserve_vram_mb/reserve_ram_mb)
    pub allocated: u64,
    pub allocations: HashMap<String, u64>,      // model_id -> bytes
}

impl DeviceResources {
    pub fn available(&self) -> u64 {
        self.total_memory
            .saturating_sub(self.reserved)
            .saturating_sub(self.allocated)
    }

    pub fn can_fit(&self, bytes: u64) -> bool {
        self.available() >= bytes
    }

    pub fn allocate(&mut self, model_id: &str, bytes: u64) -> bool {
        if self.can_fit(bytes) {
            self.allocated += bytes;
            self.allocations.insert(model_id.to_string(), bytes);
            true
        } else {
            false
        }
    }

    pub fn release(&mut self, model_id: &str) {
        if let Some(bytes) = self.allocations.remove(model_id) {
            self.allocated = self.allocated.saturating_sub(bytes);
        }
    }
}
```

### 4.4 Resource Tracker

Central resource management:

```rust
pub struct ResourceTracker {
    devices: RwLock<Vec<DeviceResources>>,
}

impl ResourceTracker {
    pub fn find_device_for(&self, vram_needed: u64) -> Option<DeviceId> {
        // Try GPUs first (faster inference)
        // Fall back to CPU if no GPU has space
        // Return None if nothing fits
    }

    pub fn allocate(&self, model_id: &str, device: DeviceId, bytes: u64) -> bool;
    pub fn release(&self, model_id: &str, device: DeviceId);
    pub fn get_usage(&self) -> ResourceUsage;
}
```

---

## 5. Process Lifecycle

### 5.1 Process Manager

Manage llama-server instances:

```rust
pub struct ProcessManager {
    processes: RwLock<HashMap<String, ProcessInfo>>,
    port_allocator: PortAllocator,
    llama_server_path: Option<PathBuf>,  // None if not found at startup
}

pub struct ProcessInfo {
    pub model_id: String,
    pub pid: u32,
    pub port: u16,
    pub device: DeviceId,
    pub started_at: Instant,
    pub health: ProcessHealth,
}

pub enum ProcessHealth {
    Starting,
    Healthy,
    Unhealthy { since: Instant, reason: String },
    Dead,
}

/// Port allocation for llama-server instances
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
    allocated: HashSet<u16>,
}

impl PortAllocator {
    pub fn new(range_start: u16, range_end: u16) -> Self {
        Self {
            range_start,
            range_end,
            allocated: HashSet::new(),
        }
    }

    pub fn next(&mut self) -> Result<u16, RiverError> {
        for port in self.range_start..=self.range_end {
            if !self.allocated.contains(&port) {
                self.allocated.insert(port);
                return Ok(port);
            }
        }
        Err(RiverError::orchestrator("No available ports in range"))
    }

    pub fn release(&mut self, port: u16) {
        self.allocated.remove(&port);
    }
}
```

### 5.2 llama-server Availability

At orchestrator startup, check if llama-server is available:

```rust
impl ProcessManager {
    pub fn new(config: &ProcessConfig) -> Self {
        let llama_server_path = Self::find_llama_server(&config.llama_server_path);

        if llama_server_path.is_none() {
            tracing::warn!(
                "llama-server not found at '{}'. Local model inference unavailable.",
                config.llama_server_path.display()
            );
        }

        Self {
            processes: RwLock::new(HashMap::new()),
            port_allocator: PortAllocator::new(config.port_range.0, config.port_range.1),
            llama_server_path,
        }
    }

    fn find_llama_server(configured_path: &Path) -> Option<PathBuf> {
        if configured_path.exists() {
            Some(configured_path.to_path_buf())
        } else {
            // Try PATH lookup
            which::which("llama-server").ok()
        }
    }

    pub fn is_available(&self) -> bool {
        self.llama_server_path.is_some()
    }
}
```

### 5.3 Spawning llama-server

```rust
impl ProcessManager {
    pub async fn spawn(&self, model: &LocalModel, device: DeviceId) -> Result<ProcessInfo> {
        let llama_server = self.llama_server_path.as_ref().ok_or_else(|| {
            RiverError::orchestrator("Local model inference unavailable: llama-server not found")
        })?;

        let port = self.port_allocator.next()?;

        let mut cmd = Command::new(llama_server);
        cmd.arg("--model").arg(&model.path)
           .arg("--port").arg(port.to_string())
           .arg("--ctx-size").arg(model.metadata.context_length.to_string());

        // Device-specific args
        match device {
            DeviceId::Gpu(idx) => {
                cmd.arg("--n-gpu-layers").arg("-1");  // All layers on GPU
                cmd.env("CUDA_VISIBLE_DEVICES", idx.to_string());
            }
            DeviceId::Cpu => {
                cmd.arg("--n-gpu-layers").arg("0");
            }
        }

        let child = cmd.spawn().map_err(|e| {
            RiverError::orchestrator(format!("Failed to spawn llama-server: {}", e))
        })?;

        // Wait for health check endpoint
        self.wait_for_ready(port).await?;

        Ok(ProcessInfo {
            model_id: model.id.clone(),
            pid: child.id(),
            port,
            device,
            started_at: Instant::now(),
            health: ProcessHealth::Healthy,
        })
    }
}
```

### 5.4 Health Monitoring

Background task checks process health:

```rust
async fn health_check_loop(manager: Arc<ProcessManager>) {
    loop {
        // Collect model IDs first to avoid holding lock during async health checks
        let model_ids: Vec<String> = {
            manager.processes.read().await.keys().cloned().collect()
        };

        for model_id in model_ids {
            let port = {
                manager.processes.read().await
                    .get(&model_id)
                    .map(|info| info.port)
            };

            if let Some(port) = port {
                let healthy = check_health(port).await;
                if !healthy {
                    manager.mark_unhealthy(&model_id).await;
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn check_health(port: u16) -> bool {
    // GET http://localhost:{port}/health
    // Returns true if 200 OK within 5 second timeout
}
```

### 5.5 Idle Timeout

Unload models after idle period:

```rust
pub struct IdleTracker {
    last_request: RwLock<HashMap<String, Instant>>,
    timeout: Duration,  // Default: 15 minutes
}

impl IdleTracker {
    pub fn record_request(&self, model_id: &str) {
        self.last_request.write().insert(model_id.to_string(), Instant::now());
    }

    pub fn idle_models(&self) -> Vec<String> {
        let now = Instant::now();
        self.last_request.read()
            .iter()
            .filter(|(_, last)| now.duration_since(**last) > self.timeout)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

async fn idle_eviction_loop(orchestrator: Arc<OrchestratorState>) {
    loop {
        for model_id in orchestrator.idle_tracker.idle_models() {
            tracing::info!("Evicting idle model: {}", model_id);
            orchestrator.unload_model(&model_id).await;
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
```

### 5.6 On-Demand Eviction

When resources needed, evict least-recently-used:

```rust
impl OrchestratorState {
    pub async fn evict_for_space(&self, needed: u64, device: DeviceId) -> Result<()> {
        // Get loaded models on this device, sorted by last_request (oldest first)
        let candidates = self.get_eviction_candidates(device).await;

        let mut freed = 0u64;
        for model_id in candidates {
            if freed >= needed {
                break;
            }
            let model_vram = self.get_model_vram(&model_id);
            self.unload_model(&model_id).await?;
            freed += model_vram;
        }

        if freed >= needed {
            Ok(())
        } else {
            Err(RiverError::orchestrator("Insufficient resources even after eviction"))
        }
    }
}
```

---

## 6. LiteLLM Integration

### 6.1 External Model Configuration

```json
{
  "external_models": [
    {
      "id": "claude-sonnet-4-20250514",
      "provider": "litellm",
      "litellm_model": "claude-sonnet-4-20250514",
      "api_base": "http://localhost:4000"
    },
    {
      "id": "gpt-4o",
      "provider": "litellm",
      "litellm_model": "gpt-4o",
      "api_base": "http://localhost:4000"
    }
  ]
}
```

### 6.2 External Model Registry

```rust
pub struct ExternalModel {
    pub id: String,              // Consistent with LocalModel.id
    pub provider: String,
    pub litellm_model: String,
    pub api_base: String,
}

impl ExternalModel {
    pub fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.api_base)
    }
}
```

### 6.3 Unified Model List

Combine local and external models:

```rust
pub enum ModelEntry {
    Local(LocalModel),
    External(ExternalModel),
}

impl OrchestratorState {
    pub fn all_models(&self) -> Vec<ModelEntry> {
        let mut models = Vec::new();
        models.extend(self.local_models.iter().map(ModelEntry::Local));
        models.extend(self.external_models.iter().map(ModelEntry::External));
        models
    }
}
```

---

## 7. API Endpoints

### 7.1 POST /model/request

Request a model be loaded and ready for inference.

**Request:**
```json
{
  "model": "qwen3-32b-q4_k_m",
  "priority": "interactive",
  "timeout_seconds": 120
}
```

**Priority levels:**
- `interactive` - User waiting, high priority, can evict lower priority
- `batch` - Background task, low priority, waits for resources

**Blocking Behavior:**
The endpoint blocks for up to `timeout_seconds` (default: 120). If the model becomes ready within that time, `status: ready` is returned with the endpoint. If still loading when timeout expires, `status: loading` is returned and the client should retry. The model continues loading in the background.

**Response (success):**
```json
{
  "status": "ready",
  "endpoint": "http://localhost:8081/v1/chat/completions",
  "model": "qwen3-32b-q4_k_m",
  "device": "gpu:0",
  "priority": "interactive"
}
```

**Response (success with swap warning):**
```json
{
  "status": "ready",
  "endpoint": "http://localhost:8082/v1/chat/completions",
  "model": "llama3-70b-q4_k_m",
  "device": "cpu",
  "priority": "interactive",
  "warning": "Model will use ~8.5GB swap. Expect slow inference due to memory pressure."
}
```

**Response (loading - timeout expired):**
```json
{
  "status": "loading",
  "model": "qwen3-32b-q4_k_m",
  "estimated_seconds": 30,
  "message": "Model still loading, retry request"
}
```

**Response (queued):**
```json
{
  "status": "queued",
  "model": "qwen3-32b-q4_k_m",
  "position": 2,
  "reason": "waiting for resources"
}
```

**Response (error):**
```json
{
  "status": "error",
  "model": "qwen3-32b-q4_k_m",
  "error": "Model requires 24GB VRAM, only 12GB available"
}
```

**Response (llama-server unavailable):**
```json
{
  "status": "error",
  "model": "qwen3-32b-q4_k_m",
  "error": "Local model inference unavailable: llama-server not found"
}
```

**Behavior:**
1. Check if model already loaded → return endpoint immediately
2. Check if model exists (local or external)
3. For external models → return LiteLLM endpoint immediately (always ready)
4. For local models:
   - Verify llama-server is available
   - Find device with sufficient resources
   - If no space and interactive priority, evict idle/lower-priority models
   - Spawn llama-server process
   - Block until health check passes or timeout expires
   - Return endpoint or loading status

### 7.2 POST /model/release

Signal that a model can be evicted if resources needed.

**Request:**
```json
{
  "model": "qwen3-32b-q4_k_m"
}
```

**Response:**
```json
{
  "acknowledged": true
}
```

**Behavior:**
- Marks model as "releasable" for eviction
- Does NOT immediately unload (idle timeout still applies)
- Allows eviction without waiting for timeout

### 7.3 GET /models/available (enhanced)

List all models with detailed status.

**Response:**
```json
{
  "local": [
    {
      "id": "qwen3-32b-q4_k_m",
      "path": "/models/qwen3-32b-q4_k_m.gguf",
      "architecture": "qwen2",
      "parameters": "32B",
      "quantization": "Q4_K_M",
      "estimated_vram_gb": 18.5,
      "status": "loaded",
      "endpoint": "http://localhost:8081/v1/chat/completions",
      "device": "gpu:0",
      "idle_seconds": 45
    },
    {
      "id": "llama3-8b-q8_0",
      "path": "/models/llama3-8b-q8_0.gguf",
      "architecture": "llama",
      "parameters": "8B",
      "quantization": "Q8_0",
      "estimated_vram_gb": 9.2,
      "status": "available"
    }
  ],
  "external": [
    {
      "id": "claude-sonnet-4-20250514",
      "provider": "litellm",
      "endpoint": "http://localhost:4000/v1/chat/completions",
      "status": "available"
    }
  ],
  "resources": {
    "devices": [
      {
        "id": "gpu:0",
        "name": "NVIDIA RTX 4090",
        "total_vram_gb": 24.0,
        "used_vram_gb": 18.5,
        "available_vram_gb": 5.5
      },
      {
        "id": "cpu",
        "total_ram_gb": 64.0,
        "available_ram_gb": 48.2,
        "total_swap_gb": 32.0,
        "available_swap_gb": 31.5
      }
    ]
  },
  "llama_server_available": true
}
```

### 7.4 GET /resources

Detailed resource usage.

**Response:**
```json
{
  "devices": [
    {
      "id": "gpu:0",
      "name": "NVIDIA RTX 4090",
      "total_vram": 25769803776,
      "used_vram": 19864223744,
      "allocations": {
        "qwen3-32b-q4_k_m": 19864223744
      }
    }
  ],
  "loaded_models": [
    {
      "model_id": "qwen3-32b-q4_k_m",
      "device": "gpu:0",
      "vram_bytes": 19864223744,
      "port": 8081,
      "pid": 12345,
      "uptime_seconds": 3600,
      "idle_seconds": 45
    }
  ],
  "llama_server_available": true
}
```

---

## 8. Configuration

### 8.1 CLI Flags

```bash
river-orchestrator \
  --port 5000 \
  --model-dirs /models,/home/user/.cache/models \
  --external-models /etc/river/external-models.json \
  --idle-timeout 900 \
  --health-threshold 120 \
  --llama-server-path /usr/bin/llama-server \
  --port-range 8080-8180 \
  --reserve-vram-mb 500 \
  --reserve-ram-mb 2000
```

### 8.2 Config File

```json
{
  "port": 5000,
  "model_dirs": ["/models"],
  "external_models": [...],
  "idle_timeout_seconds": 900,
  "health_threshold_seconds": 120,
  "llama_server": {
    "path": "/usr/bin/llama-server",
    "port_range": [8080, 8180],
    "default_ctx_size": 8192
  },
  "resources": {
    "reserve_vram_mb": 500,
    "reserve_ram_mb": 2000
  }
}
```

---

## 9. Error Handling

### 9.1 Error Strategy

Use the existing `RiverError::orchestrator(String)` pattern for all orchestrator errors. Error messages should be descriptive:

```rust
// Model not found
RiverError::orchestrator(format!("Model not found: {}", model_id))

// Insufficient resources
RiverError::orchestrator(format!(
    "Insufficient resources: model requires {} bytes, only {} available on {}",
    needed, available, device.to_api_string()
))

// Process spawn failed
RiverError::orchestrator(format!(
    "Failed to spawn llama-server for {}: {}",
    model_id, reason
))

// GGUF parse error
RiverError::orchestrator(format!(
    "Failed to parse GGUF file {}: {}",
    path.display(), reason
))

// llama-server unavailable
RiverError::orchestrator("Local model inference unavailable: llama-server not found")
```

### 9.2 Graceful Degradation

- **No GPUs detected**: Fall back to CPU-only inference, log info message
- **Insufficient VRAM**: Fall back to CPU if model doesn't fit on any GPU
- **Model would use swap**: Log warning, proceed anyway (swap is slow but works)
- **llama-server not found**: Log warning at startup, return specific error on `/model/request` for local models, external models still work
- **Model directory missing**: Log warning, skip that directory, continue with others
- **GGUF parse failure**: Log warning with path, skip that model, continue scanning
- **Process crash**: Mark unhealthy, allow re-spawn on next request

---

## 10. Testing Strategy

### 10.1 Unit Tests

- GGUF header parsing with sample files
- VRAM estimation calculations
- Resource allocation logic
- Idle timeout calculations
- Port allocation
- DeviceId serialization/deserialization
- Model ID collision handling

### 10.2 Integration Tests

- Model discovery from test directory
- Mock llama-server process spawning
- Resource tracking across allocate/release
- Eviction scenarios

### 10.3 End-to-End Tests (manual)

- Load model, verify endpoint works
- Request model when VRAM full (triggers eviction)
- Idle timeout unloads model
- Multiple models on different GPUs

---

## 11. Implementation Plan

This specification will be implemented as Plan 5, extending the minimal orchestrator from Plan 4:

1. **Task 1-3**: Model discovery (scanner, GGUF parser, registry)
2. **Task 4-6**: Resource management (GPU discovery, memory tracking, allocator)
3. **Task 7-9**: Process lifecycle (manager, spawner, health checks)
4. **Task 10-11**: Eviction (idle timeout, on-demand)
5. **Task 12-14**: API endpoints (/model/request, /model/release, enhanced /models/available)
6. **Task 15**: LiteLLM integration
7. **Task 16**: CLI and configuration updates

---

## 12. Future Extensions

After this implementation:

1. **Request queuing** - Queue requests when resources busy
2. **Priority preemption** - Interactive evicts batch jobs
3. **Model preloading** - Predictive loading based on usage patterns
4. **Multi-node** - Distribute models across machines
5. **Metrics** - Prometheus metrics for monitoring
