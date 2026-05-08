# Plan Review: TUI Adapter Implementation

**Review Date:** 2026-05-07
**Reviewer:** Gemini CLI
**Status:** Approved

## Summary

The updated implementation plan for `river-tui` addresses all critical findings from the initial review. It correctly aligns with the gateway's protocol, ensures terminal state safety with a cleanup guard, prevents UI corruption by redirecting logs to a file, and properly utilizes the `Notify` channel for async re-renders.

---

## Verification of Fixes

### 1. Protocol Mismatch / DRY Violation
- **Status:** FIXED
- **Action:** The plan now includes a specific "Protocol Notes" section and matches the `IncomingMessage` and `Author` structs to the gateway's requirements (`id` and `name` only for Author, no `is_bot`). It also correctly uses `skip_serializing_if` for optional fields.

### 2. Terminal Lifecycle & Safety
- **Status:** FIXED
- **Action:** Task 5 now implements a `run` wrapper that calls `run_inner` and ensures `disable_raw_mode()` and `LeaveAlternateScreen` are called on all exit paths, including errors and panics.

### 3. Visual Mangling from Tracing
- **Status:** FIXED
- **Action:** Task 6 now initializes tracing with a file writer (`std::fs::File::create(&config.log_file)?`) instead of the default stdout, protecting the ratatui interface from log noise.

### 4. Spec Mismatch: Unused Notify Channel
- **Status:** FIXED
- **Action:** Task 5 now uses `tokio::select!` to wait for either terminal events or the `state.notify.notified()` signal, ensuring the TUI re-renders immediately when a new message is received from the HTTP server.

### 5. Background Task Monitoring
- **Status:** FIXED
- **Action:** Task 6 now captures errors from `axum::serve` and updates a new `server_healthy` flag in the shared state, which is then displayed in the TUI status bar (Task 5).

### 6. Scrolling Logic
- **Status:** ACKNOWLEDGED / PARTIALLY FIXED
- **Action:** The plan acknowledges the wrapping limitation as a V1 constraint. It implements basic auto-follow and re-enables follow-tail when the user manually scrolls back to the bottom.

---

## Final Assessment

The plan is now robust and follows best practices for both Rust async development and TUI application design. It is ready for implementation.
