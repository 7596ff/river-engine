# river-orchestrator Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: Critical

## Summary

river-orchestrator has correct structure but critical protocol issues: role switching is NOT two-phase commit (workers not notified), no adapter feature validation, ProcessEntry uses tagged enum instead of untagged, and graceful shutdown uses SIGKILL instead of SIGTERM. Estimated effort: 3-4 days.

## Critical Issues

### Issue 1: Role switching not two-phase commit

- **Source:** Both reviews
- **Problem:** Spec requires full prepare/commit protocol with calls to both workers. Implementation just swaps batons in registry without calling `/prepare_switch` or `/commit_switch` on workers.
- **Fix:** Implement full protocol:
  ```rust
  async fn handle_switch_roles(...) {
      // 1. Acquire dyad lock
      // 2. Call POST /prepare_switch to initiator
      // 3. Call POST /prepare_switch to partner
      // 4. If either fails, send abort to successful one
      // 5. Call POST /commit_switch to both
      // 6. Update registry batons
      // 7. Push registry
      // 8. Release lock
  }
  ```
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** Test full protocol flow, test partial failure rollback

### Issue 2: No dyad lock for role switching

- **Source:** Brutal review
- **Problem:** Spec requires dyad lock to prevent concurrent switch attempts. No lock exists.
- **Fix:** Add per-dyad mutex:
  ```rust
  struct OrchestratorState {
      dyad_locks: HashMap<String, tokio::sync::Mutex<()>>,
  }
  ```
  Acquire lock before switch, release after, return 409 if busy.
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** Test concurrent switch requests get 409

### Issue 3: switch_roles hardcodes baton direction

- **Source:** First review
- **Problem:** Code assumes initiator is always actor becoming spectator. Spec says "Either worker can call switch_roles".
- **Fix:** Read current batons from registry and swap them, don't assume direction
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** Test spectator-initiated switch

### Issue 4: Race condition in partial prepare failure

- **Source:** First review
- **Problem:** If one worker prepares successfully but the other fails, no abort is sent to the prepared worker.
- **Fix:** On partial failure, send abort to workers that prepared
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** Test abort sent on partial prepare failure

### Issue 5: Graceful shutdown uses SIGKILL

- **Source:** Both reviews
- **Problem:** Spec requires SIGTERM with 5 minute grace period. Implementation uses `start_kill()` which sends SIGKILL immediately.
- **Fix:**
  ```rust
  // On Unix, send SIGTERM first
  #[cfg(unix)]
  {
      use nix::sys::signal::{kill, Signal};
      let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
  }
  // Wait up to 5 minutes
  // Then SIGKILL if still running
  ```
- **Files:** `crates/river-orchestrator/src/supervisor.rs`
- **Tests:** Manual testing (signal handling hard to unit test)

### Issue 6: No adapter feature validation

- **Source:** Both reviews
- **Problem:** Spec requires validating required features (SendMessage, ReceiveMessage). Implementation accepts features as-is.
- **Fix:**
  ```rust
  fn validate_adapter_features(features: &[u16]) -> Result<Vec<FeatureId>, Error> {
      let parsed = features.iter().map(|f| FeatureId::try_from(*f)).collect()?;
      if !parsed.contains(&FeatureId::SendMessage) {
          return Err(MissingFeature(FeatureId::SendMessage));
      }
      if !parsed.contains(&FeatureId::ReceiveMessage) {
          return Err(MissingFeature(FeatureId::ReceiveMessage));
      }
      Ok(parsed)
  }
  ```
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** Test rejection of adapters missing required features

## Important Issues

### Issue 7: ProcessEntry uses tagged instead of untagged

- **Source:** Both reviews
- **Problem:** Spec shows `#[serde(untagged)]` for ProcessEntry. Implementation in river-protocol uses `#[serde(tag = "entry_type")]`. This produces different JSON.
- **Fix:** Either change to untagged or update spec. Recommend updating spec since tagged is more explicit.
- **Files:** `crates/river-protocol/src/registry.rs` or spec
- **Tests:** Serde roundtrip test to verify format

### Issue 8: Config field names differ from spec

- **Source:** First review
- **Problem:** Spec has `left_starts_as: Baton`, implementation has `initial_actor: Side`. Semantically similar but breaks config file compatibility.
- **Fix:** Update config field names to match spec
- **Files:** `crates/river-orchestrator/src/config.rs`
- **Tests:** Config parsing test

### Issue 9: Blocking file I/O in async context

- **Source:** First review
- **Problem:** `std::fs::read_to_string(path)` in config loading blocks the runtime.
- **Fix:** Use `tokio::fs::read_to_string`
- **Files:** `crates/river-orchestrator/src/config.rs`
- **Tests:** N/A

### Issue 10: No validation of model references at startup

- **Source:** Brutal review
- **Problem:** Spec says unknown model reference should exit with error. No validation exists.
- **Fix:** Validate `left_model` and `right_model` exist in models map during config loading
- **Files:** `crates/river-orchestrator/src/config.rs`
- **Tests:** Test invalid model reference rejection

### Issue 11: Missing thiserror dependency

- **Source:** Both reviews
- **Problem:** Spec lists thiserror but it's not used. Errors implemented manually.
- **Fix:** Add thiserror and use `#[derive(Error)]`
- **Files:** `crates/river-orchestrator/Cargo.toml`, error types
- **Tests:** N/A

### Issue 12: Uses river-adapter instead of river-protocol

- **Source:** Brutal review
- **Problem:** `use river_adapter::Side` instead of `river_protocol::Side`.
- **Fix:** Use river-protocol consistently
- **Files:** `crates/river-orchestrator/src/http.rs`
- **Tests:** N/A

## Minor Issues

### Issue 13: WorkerOutput has extra fields

- **Source:** Brutal review
- **Problem:** Implementation adds `dyad` and `side` fields not in spec. These are useful (self-identifying) but diverge from spec.
- **Fix:** Update spec to include these fields (beneficial deviation)
- **Files:** Spec update
- **Tests:** N/A

### Issue 14: Hardcoded timeouts throughout

- **Source:** First review
- **Problem:** 2s, 5s, 10s, 60s timeouts hardcoded in various places.
- **Fix:** Consolidate into constants or config
- **Files:** Multiple files
- **Tests:** N/A

### Issue 15: Dead code warnings

- **Source:** First review
- **Problem:** `get_embed_endpoint`, `next_wake_time`, `kill`, `KillFailed` are unused.
- **Fix:** Remove or use these
- **Files:** Various
- **Tests:** N/A

### Issue 16: No supervisor tests

- **Source:** Both reviews
- **Problem:** Only respawn.rs has tests. Supervisor.rs has none.
- **Fix:** Add integration tests using mock processes
- **Files:** `crates/river-orchestrator/src/supervisor.rs`
- **Tests:** Process spawn, health check, respawn tests

## Spec Updates Needed

1. Clarify ProcessEntry serde strategy (tagged vs untagged)
2. Add `dyad` and `side` fields to WorkerOutput spec
3. Document that config uses `initial_actor: Side` instead of `left_starts_as: Baton`

## Verification Checklist

- [ ] Two-phase commit fully implemented for role switching
- [ ] Dyad lock prevents concurrent switches
- [ ] Baton swap reads actual values, doesn't assume direction
- [ ] Abort sent on partial prepare failure
- [ ] Graceful shutdown sends SIGTERM first
- [ ] Adapter features validated (SendMessage, ReceiveMessage required)
- [ ] ProcessEntry serde format documented/reconciled
- [ ] Model references validated at startup
- [ ] Health check interval works correctly
- [ ] Supervisor tests added
