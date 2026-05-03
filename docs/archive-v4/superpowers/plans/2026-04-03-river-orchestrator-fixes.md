# river-orchestrator Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical protocol issues in river-orchestrator including two-phase commit role switching, graceful shutdown with SIGTERM, proper baton swap logic, and abort-on-partial-failure handling.

**Architecture:** The orchestrator manages workers, adapters, and embed services through HTTP endpoints for registration and role switching. The supervisor handles process lifecycle with health checks and respawning. Role switching requires a two-phase commit protocol coordinating both workers in a dyad, protected by per-dyad locks.

**Tech Stack:** Rust, Tokio async runtime, Axum HTTP framework, reqwest HTTP client, serde JSON, Unix signals (nix crate for SIGTERM)

---

## File Structure

```
crates/river-orchestrator/
  Cargo.toml           # Add nix + thiserror dependencies
  src/
    main.rs            # Entry point (minor changes)
    config.rs          # Add async file loading
    http.rs            # Two-phase commit, baton swap, abort handling
    supervisor.rs      # SIGTERM graceful shutdown
    registry.rs        # Add get_worker_baton helper
    respawn.rs         # No changes needed
    model.rs           # No changes needed
```

---

## Task 1: Add Dependencies to Cargo.toml

**Goal:** Add `nix` for Unix signal handling and `thiserror` for error derive macros.

**Files:** `crates/river-orchestrator/Cargo.toml`

### Steps

- [ ] **1.1** Add nix dependency for SIGTERM signal handling

Open `crates/river-orchestrator/Cargo.toml` and add after the `regex` line:

```toml
nix = { version = "0.29", features = ["signal"] }
thiserror = "1"
```

- [ ] **1.2** Verify the crate compiles with new dependencies

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **1.3** Commit changes

```bash
git add crates/river-orchestrator/Cargo.toml
git commit -m "chore(orchestrator): add nix and thiserror dependencies"
```

---

## Task 2: Add get_worker_baton Helper to Registry

**Goal:** Add a method to retrieve a worker's current baton from the registry, needed for proper baton swapping.

**Files:** `crates/river-orchestrator/src/registry.rs`

### Steps

- [ ] **2.1** Add `get_worker_baton` method to RegistryState

In `crates/river-orchestrator/src/registry.rs`, add this method to the `impl RegistryState` block after `get_partner_endpoint`:

```rust
    /// Get worker's current baton.
    pub fn get_worker_baton(&self, dyad: &str, side: &Side) -> Option<Baton> {
        let key = WorkerKey {
            dyad: dyad.to_string(),
            side: side.clone(),
        };
        self.workers.get(&key).and_then(|e| {
            if let ProcessEntry::Worker { baton, .. } = e {
                Some(baton.clone())
            } else {
                None
            }
        })
    }
```

- [ ] **2.2** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **2.3** Commit changes

```bash
git add crates/river-orchestrator/src/registry.rs
git commit -m "feat(orchestrator): add get_worker_baton helper to registry"
```

---

## Task 3: Fix Graceful Shutdown to Use SIGTERM

**Goal:** Replace immediate SIGKILL with SIGTERM followed by grace period, then SIGKILL for stragglers.

**Files:** `crates/river-orchestrator/src/supervisor.rs`

### Steps

- [ ] **3.1** Add Unix signal imports at the top of supervisor.rs

In `crates/river-orchestrator/src/supervisor.rs`, add after the existing imports:

```rust
#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;
```

- [ ] **3.2** Replace the `terminate_all` method with SIGTERM-first implementation

Replace the existing `terminate_all` method:

```rust
    /// Send SIGTERM to all processes (or kill on non-Unix).
    pub async fn terminate_all(&mut self) {
        for (key, handle) in &mut self.processes {
            // For graceful shutdown, we just kill the process
            // Workers should handle this by writing summary
            if let Err(e) = handle.child.start_kill() {
                tracing::warn!("Failed to kill {:?}: {}", key, e);
            }
        }
    }
```

With:

```rust
    /// Send SIGTERM to all processes for graceful shutdown.
    /// On non-Unix platforms, falls back to immediate kill.
    pub async fn terminate_all(&mut self) {
        for (key, handle) in &self.processes {
            #[cfg(unix)]
            {
                if let Some(pid) = handle.child.id() {
                    let pid = Pid::from_raw(pid as i32);
                    if let Err(e) = kill(pid, Signal::SIGTERM) {
                        tracing::warn!("Failed to send SIGTERM to {:?}: {}", key, e);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, we can't send SIGTERM, so just mark for kill
                tracing::info!("Non-Unix platform, will force kill {:?}", key);
            }
        }
    }
```

- [ ] **3.3** Update the `shutdown` method to use the 5-minute grace period from spec

Replace the existing `shutdown` method:

```rust
    /// Wait for all processes to exit with timeout, then kill stragglers.
    pub async fn shutdown(&mut self, timeout: Duration) {
        self.terminate_all().await;

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if self.processes.is_empty() {
                break;
            }

            if tokio::time::Instant::now() > deadline {
                tracing::warn!("Shutdown timeout, killing remaining processes");
                for (_, handle) in &mut self.processes {
                    let _ = handle.child.kill().await;
                }
                break;
            }

            // Check for exited processes
            let mut exited = Vec::new();
            for (key, handle) in &mut self.processes {
                match handle.child.try_wait() {
                    Ok(Some(_)) => exited.push(key.clone()),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Error checking process {:?}: {}", key, e);
                        exited.push(key.clone());
                    }
                }
            }
            for key in exited {
                self.processes.remove(&key);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
```

With:

```rust
    /// Wait for all processes to exit with timeout, then SIGKILL stragglers.
    /// Spec requires 5 minute grace period for SIGTERM.
    pub async fn shutdown(&mut self, timeout: Duration) {
        tracing::info!("Sending SIGTERM to all processes, waiting up to {:?}", timeout);
        self.terminate_all().await;

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if self.processes.is_empty() {
                tracing::info!("All processes exited gracefully");
                break;
            }

            if tokio::time::Instant::now() > deadline {
                tracing::warn!(
                    "Shutdown timeout after {:?}, sending SIGKILL to {} remaining processes",
                    timeout,
                    self.processes.len()
                );
                for (key, handle) in &mut self.processes {
                    if let Err(e) = handle.child.kill().await {
                        tracing::warn!("Failed to SIGKILL {:?}: {}", key, e);
                    }
                }
                self.processes.clear();
                break;
            }

            // Check for exited processes
            let mut exited = Vec::new();
            for (key, handle) in &mut self.processes {
                match handle.child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::debug!("Process {:?} exited with status {:?}", key, status);
                        exited.push(key.clone());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Error checking process {:?}: {}", key, e);
                        exited.push(key.clone());
                    }
                }
            }
            for key in exited {
                self.processes.remove(&key);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
```

- [ ] **3.4** Remove the unused `kill` method and `KillFailed` error variant

The `kill` method is unused (dead code). Remove it from the `impl Supervisor` block:

```rust
    /// Kill a process.
    pub async fn kill(&mut self, key: &ProcessKey) -> Result<(), SupervisorError> {
        if let Some(mut handle) = self.processes.remove(key) {
            handle.child.kill().await.map_err(SupervisorError::KillFailed)?;
        }
        Ok(())
    }
```

Also remove `KillFailed` from the `SupervisorError` enum:

```rust
    KillFailed(std::io::Error),
```

And remove it from the `Display` impl:

```rust
            SupervisorError::KillFailed(e) => write!(f, "Failed to kill process: {}", e),
```

- [ ] **3.5** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **3.6** Commit changes

```bash
git add crates/river-orchestrator/src/supervisor.rs
git commit -m "fix(orchestrator): use SIGTERM with 5-min grace period for graceful shutdown"
```

---

## Task 4: Fix Async Config Loading

**Goal:** Replace blocking `std::fs::read_to_string` with `tokio::fs::read_to_string`.

**Files:** `crates/river-orchestrator/src/config.rs`, `crates/river-orchestrator/src/main.rs`

### Steps

- [ ] **4.1** Convert `load_config` to async in config.rs

Replace the existing `load_config` function:

```rust
/// Load configuration from file with env var substitution.
pub fn load_config(path: &std::path::Path) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let resolved = substitute_env_vars(&content)?;
    let config: Config = serde_json::from_str(&resolved)?;
    validate_config(&config)?;
    Ok(config)
}
```

With:

```rust
/// Load configuration from file with env var substitution.
pub async fn load_config(path: &std::path::Path) -> Result<Config, ConfigError> {
    let content = tokio::fs::read_to_string(path).await?;
    let resolved = substitute_env_vars(&content)?;
    let config: Config = serde_json::from_str(&resolved)?;
    validate_config(&config)?;
    Ok(config)
}
```

- [ ] **4.2** Update main.rs to await the config loading

In `crates/river-orchestrator/src/main.rs`, change line ~54:

```rust
    let mut config = load_config(&args.config)?;
```

To:

```rust
    let mut config = load_config(&args.config).await?;
```

- [ ] **4.3** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **4.4** Commit changes

```bash
git add crates/river-orchestrator/src/config.rs crates/river-orchestrator/src/main.rs
git commit -m "fix(orchestrator): use async file I/O for config loading"
```

---

## Task 5: Fix Import to Use river-protocol Instead of river-adapter

**Goal:** Use types from river-protocol consistently as per spec.

**Files:** `crates/river-orchestrator/src/http.rs`

### Steps

- [ ] **5.1** Update imports in http.rs

In `crates/river-orchestrator/src/http.rs`, change line 15:

```rust
use river_adapter::{Baton, FeatureId, Ground, Side};
```

To:

```rust
use river_adapter::FeatureId;
use river_protocol::{Baton, Ground, Side};
```

- [ ] **5.2** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **5.3** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs
git commit -m "refactor(orchestrator): use types from river-protocol consistently"
```

---

## Task 6: Fix Baton Swap to Read Actual Values

**Goal:** The `handle_switch_roles` function currently hardcodes that the initiator becomes spectator. It should read actual batons from registry and swap them.

**Files:** `crates/river-orchestrator/src/http.rs`

### Steps

- [ ] **6.1** Update the baton swap logic in `handle_switch_roles`

In `crates/river-orchestrator/src/http.rs`, replace the baton swap section (lines ~597-612):

```rust
    // Update registry with swapped batons
    let (your_new_baton, partner_new_baton) = {
        let mut reg = state.registry.write().await;
        let partner_side = match req.side {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        };

        // Swap batons
        reg.update_worker_baton(&req.dyad, &req.side, Baton::Spectator);
        reg.update_worker_baton(&req.dyad, &partner_side, Baton::Actor);

        // The initiator becomes spectator, partner becomes actor
        // (Assuming initiator was actor requesting the switch)
        (Baton::Spectator, Baton::Actor)
    };
```

With:

```rust
    // Update registry with swapped batons - read actual values and swap them
    let (your_new_baton, partner_new_baton) = {
        let mut reg = state.registry.write().await;
        let partner_side = req.side.opposite();

        // Get current batons
        let initiator_baton = reg.get_worker_baton(&req.dyad, &req.side);
        let partner_baton = reg.get_worker_baton(&req.dyad, &partner_side);

        match (initiator_baton, partner_baton) {
            (Some(init_baton), Some(part_baton)) => {
                // Swap: initiator gets partner's baton, partner gets initiator's baton
                let new_initiator_baton = part_baton.clone();
                let new_partner_baton = init_baton;

                reg.update_worker_baton(&req.dyad, &req.side, new_initiator_baton.clone());
                reg.update_worker_baton(&req.dyad, &partner_side, new_partner_baton.clone());

                (new_initiator_baton, new_partner_baton)
            }
            _ => {
                // This shouldn't happen if both workers are registered
                // but handle gracefully by defaulting to previous behavior
                tracing::warn!("Could not read batons for dyad {}, using default swap", req.dyad);
                reg.update_worker_baton(&req.dyad, &req.side, Baton::Spectator);
                reg.update_worker_baton(&req.dyad, &partner_side, Baton::Actor);
                (Baton::Spectator, Baton::Actor)
            }
        }
    };
```

- [ ] **6.2** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **6.3** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs
git commit -m "fix(orchestrator): read actual batons and swap them in role switch"
```

---

## Task 7: Implement Abort on Partial Prepare Failure

**Goal:** If one worker prepares successfully but the other fails, send abort to the prepared worker.

**Files:** `crates/river-orchestrator/src/http.rs`

### Steps

- [ ] **7.1** Replace `prepare_both` function with one that tracks individual results

Replace the existing `prepare_both` function:

```rust
async fn prepare_both(client: &reqwest::Client, initiator: &str, partner: &str) -> bool {
    let prep_body = serde_json::json!({"phase": "prepare"});

    let init_result = client
        .post(format!("{}/prepare_switch", initiator))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let partner_result = client
        .post(format!("{}/prepare_switch", partner))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    matches!(
        (init_result, partner_result),
        (Ok(r1), Ok(r2)) if r1.status().is_success() && r2.status().is_success()
    )
}
```

With:

```rust
/// Result of preparing workers for role switch.
enum PrepareResult {
    /// Both workers prepared successfully
    BothPrepared,
    /// Initiator prepared but partner failed - need to abort initiator
    InitiatorPreparedPartnerFailed,
    /// Partner prepared but initiator failed - need to abort partner
    PartnerPreparedInitiatorFailed,
    /// Both failed to prepare
    BothFailed,
}

async fn prepare_both(client: &reqwest::Client, initiator: &str, partner: &str) -> PrepareResult {
    let prep_body = serde_json::json!({"phase": "prepare"});

    // Prepare initiator first
    let init_result = client
        .post(format!("{}/prepare_switch", initiator))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let initiator_ok = matches!(&init_result, Ok(r) if r.status().is_success());

    // Prepare partner
    let partner_result = client
        .post(format!("{}/prepare_switch", partner))
        .json(&prep_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let partner_ok = matches!(&partner_result, Ok(r) if r.status().is_success());

    match (initiator_ok, partner_ok) {
        (true, true) => PrepareResult::BothPrepared,
        (true, false) => PrepareResult::InitiatorPreparedPartnerFailed,
        (false, true) => PrepareResult::PartnerPreparedInitiatorFailed,
        (false, false) => PrepareResult::BothFailed,
    }
}
```

- [ ] **7.2** Add `send_abort` helper function

Add this function after `prepare_both`:

```rust
/// Send abort to a worker that prepared but whose partner failed.
async fn send_abort(client: &reqwest::Client, endpoint: &str) {
    let abort_body = serde_json::json!({"phase": "abort"});
    let result = client
        .post(format!("{}/abort_switch", endpoint))
        .json(&abort_body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    if let Err(e) = result {
        tracing::warn!("Failed to send abort to {}: {}", endpoint, e);
    }
}
```

- [ ] **7.3** Update `handle_switch_roles` to use new prepare logic with abort handling

Replace the Phase 1 section in `handle_switch_roles` (lines ~561-577):

```rust
    // Phase 1: Prepare both workers
    let prepare_result = prepare_both(
        &state.client,
        &initiator_endpoint,
        &partner_endpoint,
    ).await;

    if !prepare_result {
        release_lock(&state, &req.dyad).await;
        return Err((
            StatusCode::CONFLICT,
            Json(SwitchRolesError {
                error: "partner_busy".into(),
                message: Some("Partner worker is mid-operation".into()),
            }),
        ));
    }
```

With:

```rust
    // Phase 1: Prepare both workers
    let prepare_result = prepare_both(
        &state.client,
        &initiator_endpoint,
        &partner_endpoint,
    ).await;

    match prepare_result {
        PrepareResult::BothPrepared => {
            // Continue to commit phase
        }
        PrepareResult::InitiatorPreparedPartnerFailed => {
            // Abort the initiator since partner failed
            send_abort(&state.client, &initiator_endpoint).await;
            release_lock(&state, &req.dyad).await;
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "partner_busy".into(),
                    message: Some("Partner worker is mid-operation, switch aborted".into()),
                }),
            ));
        }
        PrepareResult::PartnerPreparedInitiatorFailed => {
            // Abort the partner since initiator failed
            send_abort(&state.client, &partner_endpoint).await;
            release_lock(&state, &req.dyad).await;
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "initiator_busy".into(),
                    message: Some("Initiator worker is mid-operation, switch aborted".into()),
                }),
            ));
        }
        PrepareResult::BothFailed => {
            release_lock(&state, &req.dyad).await;
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "workers_busy".into(),
                    message: Some("Both workers are mid-operation".into()),
                }),
            ));
        }
    }
```

- [ ] **7.4** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **7.5** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs
git commit -m "fix(orchestrator): send abort on partial prepare failure in role switch"
```

---

## Task 8: Improve Dyad Lock Implementation

**Goal:** Replace the `HashMap<String, bool>` with proper `tokio::sync::Mutex` per dyad for more robust locking.

**Files:** `crates/river-orchestrator/src/http.rs`, `crates/river-orchestrator/src/main.rs`

### Steps

- [ ] **8.1** Update AppState to use proper Mutex per dyad

In `crates/river-orchestrator/src/http.rs`, change the `dyad_locks` field type in `AppState`:

```rust
/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub registry: SharedRegistry,
    pub supervisor: SharedSupervisor,
    pub respawn: SharedRespawnManager,
    pub client: reqwest::Client,
    pub dyad_locks: Arc<RwLock<HashMap<String, bool>>>,
    pub orchestrator_url: String,
}
```

To:

```rust
/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub registry: SharedRegistry,
    pub supervisor: SharedSupervisor,
    pub respawn: SharedRespawnManager,
    pub client: reqwest::Client,
    pub dyad_locks: Arc<RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    pub orchestrator_url: String,
}
```

- [ ] **8.2** Update lock acquisition in `handle_switch_roles`

Replace the lock acquisition section (lines ~510-523):

```rust
    // Acquire dyad lock
    {
        let mut locks = state.dyad_locks.write().await;
        if *locks.get(&req.dyad).unwrap_or(&false) {
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "switch_in_progress".into(),
                    message: Some("Another switch is already in progress".into()),
                }),
            ));
        }
        locks.insert(req.dyad.clone(), true);
    }
```

With:

```rust
    // Get or create the dyad lock
    let dyad_lock = {
        let mut locks = state.dyad_locks.write().await;
        locks.entry(req.dyad.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };

    // Try to acquire the lock without blocking
    let _guard = match dyad_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Err((
                StatusCode::CONFLICT,
                Json(SwitchRolesError {
                    error: "switch_in_progress".into(),
                    message: Some("Another switch is already in progress".into()),
                }),
            ));
        }
    };
```

- [ ] **8.3** Remove the `release_lock` helper function and its calls

Remove the `release_lock` function entirely:

```rust
async fn release_lock(state: &AppState, dyad: &str) {
    let mut locks = state.dyad_locks.write().await;
    locks.remove(dyad);
}
```

Then remove all calls to `release_lock` in `handle_switch_roles` - the lock is now automatically released when `_guard` goes out of scope at the end of the function. There are 4 calls to remove:

1. After partner not found error (line ~536)
2. After initiator not found error (line ~549)
3. After prepare failure cases (multiple in the match block)
4. After commit failure (line ~588)
5. After successful switch (line ~618)

The function will now just return errors directly without calling `release_lock`.

- [ ] **8.4** Update main.rs initialization

In `crates/river-orchestrator/src/main.rs`, the initialization is already correct since it creates an empty HashMap. No changes needed.

- [ ] **8.5** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **8.6** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs
git commit -m "refactor(orchestrator): use proper Mutex for dyad locks with RAII cleanup"
```

---

## Task 9: Add Tests for Role Switching Protocol

**Goal:** Add unit tests for the two-phase commit protocol, baton swapping, and abort handling.

**Files:** `crates/river-orchestrator/src/http.rs`

### Steps

- [ ] **9.1** Add test module at the bottom of http.rs

Add this test module at the end of `crates/river-orchestrator/src/http.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_switch_roles_request_serde() {
        let req = SwitchRolesRequest {
            dyad: "test-dyad".into(),
            side: Side::Left,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("test-dyad"));
        assert!(json.contains("left"));
    }

    #[test]
    fn test_switch_roles_response_serde() {
        let resp = SwitchRolesResponse {
            switched: true,
            your_new_baton: Baton::Spectator,
            partner_new_baton: Baton::Actor,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""switched":true"#));
        assert!(json.contains(r#""your_new_baton":"spectator""#));
        assert!(json.contains(r#""partner_new_baton":"actor""#));
    }

    #[test]
    fn test_switch_roles_error_serde() {
        let err = SwitchRolesError {
            error: "switch_in_progress".into(),
            message: Some("Another switch is already in progress".into()),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("switch_in_progress"));
        assert!(json.contains("Another switch"));

        // Test without message
        let err_no_msg = SwitchRolesError {
            error: "partner_busy".into(),
            message: None,
        };
        let json = serde_json::to_string(&err_no_msg).unwrap();
        assert!(json.contains("partner_busy"));
        assert!(!json.contains("message"));
    }

    #[test]
    fn test_prepare_result_variants() {
        // Just verify the enum exists and variants are correct
        let _both = PrepareResult::BothPrepared;
        let _init = PrepareResult::InitiatorPreparedPartnerFailed;
        let _part = PrepareResult::PartnerPreparedInitiatorFailed;
        let _none = PrepareResult::BothFailed;
    }
}
```

- [ ] **9.2** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-orchestrator
```

- [ ] **9.3** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs
git commit -m "test(orchestrator): add tests for role switching protocol types"
```

---

## Task 10: Add Tests for Registry Baton Methods

**Goal:** Add tests for the new `get_worker_baton` method and baton update logic.

**Files:** `crates/river-orchestrator/src/registry.rs`

### Steps

- [ ] **10.1** Add test module to registry.rs

Add this test module at the end of `crates/river-orchestrator/src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use river_protocol::Channel;

    fn test_ground() -> Ground {
        Ground {
            name: "Test User".into(),
            id: "user123".into(),
            channel: Channel {
                adapter: "discord".into(),
                id: "ch123".into(),
                name: Some("general".into()),
            },
        }
    }

    #[test]
    fn test_get_worker_baton() {
        let mut state = RegistryState::new();

        // Initially no worker
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), None);

        // Register a worker as Actor
        state.register_worker(
            "dyad1".into(),
            Side::Left,
            "http://localhost:3001".into(),
            Baton::Actor,
            "gpt-4".into(),
            test_ground(),
        );

        // Should get Actor baton
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Actor));

        // Other side still None
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), None);
    }

    #[test]
    fn test_baton_swap() {
        let mut state = RegistryState::new();
        let ground = test_ground();

        // Register both workers
        state.register_worker(
            "dyad1".into(),
            Side::Left,
            "http://localhost:3001".into(),
            Baton::Actor,
            "gpt-4".into(),
            ground.clone(),
        );
        state.register_worker(
            "dyad1".into(),
            Side::Right,
            "http://localhost:3002".into(),
            Baton::Spectator,
            "gpt-4".into(),
            ground,
        );

        // Verify initial state
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Actor));
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), Some(Baton::Spectator));

        // Swap batons
        let left_baton = state.get_worker_baton("dyad1", &Side::Left).unwrap();
        let right_baton = state.get_worker_baton("dyad1", &Side::Right).unwrap();

        state.update_worker_baton("dyad1", &Side::Left, right_baton);
        state.update_worker_baton("dyad1", &Side::Right, left_baton);

        // Verify swapped state
        assert_eq!(state.get_worker_baton("dyad1", &Side::Left), Some(Baton::Spectator));
        assert_eq!(state.get_worker_baton("dyad1", &Side::Right), Some(Baton::Actor));
    }

    #[test]
    fn test_update_worker_baton_nonexistent() {
        let mut state = RegistryState::new();

        // Should return false for nonexistent worker
        assert!(!state.update_worker_baton("dyad1", &Side::Left, Baton::Actor));
    }
}
```

- [ ] **10.2** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-orchestrator
```

- [ ] **10.3** Commit changes

```bash
git add crates/river-orchestrator/src/registry.rs
git commit -m "test(orchestrator): add tests for registry baton operations"
```

---

## Task 11: Add Supervisor Shutdown Tests

**Goal:** Add basic tests for supervisor process management.

**Files:** `crates/river-orchestrator/src/supervisor.rs`

### Steps

- [ ] **11.1** Add test module to supervisor.rs

Add this test module at the end of `crates/river-orchestrator/src/supervisor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_key_equality() {
        let key1 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Left,
        };
        let key2 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Left,
        };
        let key3 = ProcessKey::Worker {
            dyad: "dyad1".into(),
            side: Side::Right,
        };

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_process_key_adapter() {
        let key1 = ProcessKey::Adapter {
            dyad: "dyad1".into(),
            adapter_type: "discord".into(),
        };
        let key2 = ProcessKey::Adapter {
            dyad: "dyad1".into(),
            adapter_type: "slack".into(),
        };

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_supervisor_new() {
        let sup = Supervisor::new();
        assert!(sup.endpoints_for_health_check().is_empty());
    }

    #[test]
    fn test_supervisor_set_endpoint_nonexistent() {
        let mut sup = Supervisor::new();
        // Setting endpoint for nonexistent process should be a no-op
        sup.set_endpoint(
            &ProcessKey::Worker {
                dyad: "dyad1".into(),
                side: Side::Left,
            },
            "http://localhost:3001".into(),
        );
        // Should not crash, just do nothing
        assert!(sup.endpoints_for_health_check().is_empty());
    }

    #[test]
    fn test_supervisor_failure_tracking() {
        let mut sup = Supervisor::new();
        let key = ProcessKey::Embed { name: "embed".into() };

        // Recording failure for nonexistent process returns 0
        assert_eq!(sup.record_failure(&key), 0);

        // Reset failures for nonexistent process is a no-op
        sup.reset_failures(&key);
    }

    #[test]
    fn test_supervisor_error_display() {
        let err = SupervisorError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "binary not found",
        ));
        let display = format!("{}", err);
        assert!(display.contains("Failed to spawn process"));
        assert!(display.contains("binary not found"));
    }
}
```

- [ ] **11.2** Run tests

```bash
cd /home/cassie/river-engine && cargo test -p river-orchestrator
```

- [ ] **11.3** Commit changes

```bash
git add crates/river-orchestrator/src/supervisor.rs
git commit -m "test(orchestrator): add supervisor unit tests"
```

---

## Task 12: Add Constants for Hardcoded Timeouts

**Goal:** Extract hardcoded timeout values into constants for maintainability.

**Files:** `crates/river-orchestrator/src/http.rs`

### Steps

- [ ] **12.1** Add timeout constants at the top of http.rs

After the imports in `crates/river-orchestrator/src/http.rs`, add:

```rust
/// Timeout for prepare/commit/abort requests during role switching.
const SWITCH_PHASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Timeout for registry push operations.
const REGISTRY_PUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
```

- [ ] **12.2** Update `prepare_both` to use the constant

Replace the two `.timeout(std::time::Duration::from_secs(5))` calls in `prepare_both` with `.timeout(SWITCH_PHASE_TIMEOUT)`.

- [ ] **12.3** Update `commit_both` to use the constant

Replace the two `.timeout(std::time::Duration::from_secs(5))` calls in `commit_both` with `.timeout(SWITCH_PHASE_TIMEOUT)`.

- [ ] **12.4** Update `send_abort` to use the constant

Replace `.timeout(std::time::Duration::from_secs(5))` in `send_abort` with `.timeout(SWITCH_PHASE_TIMEOUT)`.

- [ ] **12.5** Update `push_registry` in registry.rs

In `crates/river-orchestrator/src/registry.rs`, add a constant and use it:

After the imports, add:

```rust
/// Timeout for registry push operations.
const REGISTRY_PUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
```

Replace `.timeout(std::time::Duration::from_secs(5))` with `.timeout(REGISTRY_PUSH_TIMEOUT)`.

- [ ] **12.6** Verify compilation

```bash
cd /home/cassie/river-engine && cargo check -p river-orchestrator
```

- [ ] **12.7** Commit changes

```bash
git add crates/river-orchestrator/src/http.rs crates/river-orchestrator/src/registry.rs
git commit -m "refactor(orchestrator): extract hardcoded timeouts into constants"
```

---

## Task 13: Final Verification and Cleanup

**Goal:** Run all tests and verify the implementation is complete.

### Steps

- [ ] **13.1** Run full test suite

```bash
cd /home/cassie/river-engine && cargo test -p river-orchestrator
```

- [ ] **13.2** Run clippy for lint checks

```bash
cd /home/cassie/river-engine && cargo clippy -p river-orchestrator -- -D warnings
```

- [ ] **13.3** Verify no dead code warnings

```bash
cd /home/cassie/river-engine && cargo build -p river-orchestrator 2>&1 | grep -i "warning.*dead_code" || echo "No dead code warnings"
```

- [ ] **13.4** Run cargo fmt

```bash
cd /home/cassie/river-engine && cargo fmt -p river-orchestrator
```

- [ ] **13.5** Final commit if any formatting changes

```bash
git add -A && git diff --cached --quiet || git commit -m "style(orchestrator): apply cargo fmt"
```

---

## Verification Checklist

After completing all tasks, verify:

- [ ] Two-phase commit fully implemented for role switching (prepare both, then commit both)
- [ ] Dyad lock prevents concurrent switches (returns 409 Conflict)
- [ ] Baton swap reads actual values from registry, doesn't assume direction
- [ ] Abort sent on partial prepare failure
- [ ] Graceful shutdown sends SIGTERM first, waits 5 minutes, then SIGKILL
- [ ] Config loading uses async tokio::fs::read_to_string
- [ ] Types imported from river-protocol consistently
- [ ] All tests pass
- [ ] No clippy warnings

---

## Summary of Changes

| File | Changes |
|------|---------|
| `Cargo.toml` | Add nix and thiserror dependencies |
| `config.rs` | Convert load_config to async |
| `main.rs` | Await config loading |
| `http.rs` | Two-phase commit, proper baton swap, abort handling, Mutex locks, tests |
| `supervisor.rs` | SIGTERM graceful shutdown, remove dead code, tests |
| `registry.rs` | Add get_worker_baton helper, timeout constant, tests |
