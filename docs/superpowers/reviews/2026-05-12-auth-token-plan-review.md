# Review: Auth Token Implementation Plan (2026-05-12)

## 1. Executive Summary

The proposed implementation plan for Auth Token support is **high quality, comprehensive, and aligns perfectly with the updated specification**. It systematically addresses all 16 critical issues raised in the previous `auth-token-review.md`, transforming a problematic design into a robust, service-wide authentication layer.

The plan correctly identifies the need for both **inbound** authentication (validating requests) and **outbound** authentication (attaching tokens to requests between services), ensuring that the entire ecosystem remains functional after auth is enabled.

## 2. Review against Spec

The plan follows the specification accurately:
- **Shared Module:** Implements `river_core::auth` as specified.
- **Dependency Management:** Adds `dotenvy` to service crates and `reqwest` to `river-core` to support the shared client builder.
- **State Changes:** Migrates `auth_token` from `Option<String>` to `String` across all services.
- **Migration Path:** Maintains `--auth-token-file` as a fallback for the gateway, preventing breaking changes for existing deployments.
- **Outbound Auth:** Centralizes outbound auth via a pre-configured `reqwest::Client` in `river-core`, which is then shared across all services and clients (Heartbeat, GatewayClient, etc.).

## 3. Strengths of the Plan

- **Atomic Commits:** The plan breaks the work into logical, testable tasks with suggested commit messages.
- **Test-Driven:** Includes explicit steps for writing and running tests for the shared module and individual services.
- **Correct Scoping:** Identifies specific handlers that need `HeaderMap` parameters, including administrative routes in the Discord adapter (`POST /channels`, `DELETE /channels/{id}`) that were previously overlooked.
- **Error Handling:** Improves heartbeat error reporting by specifically logging 401 Unauthorized errors instead of swallowing them.

## 4. Minor Observations & Suggestions

### 4.1 Constant-Time Comparison
The current `validate_bearer` implementation uses standard string comparison (`token == expected`). While acceptable for a localhost/VPN-scoped deployment, using a constant-time comparison (e.g., via the `subtle` crate) would harden the system against timing attacks if exposed to a wider network.
*Suggestion: Consider adding `subtle` dependency to `river-core` for `ConstantTimeEq` comparison in a future hardening pass.*

### 4.2 `river-core` Dependencies
The specification noted that `river-core` should remain lean. However, moving `build_authed_client` to `river-core` requires adding `reqwest` as a dependency. This is a pragmatic trade-off for consistency, but it does increase the dependency surface of the core crate.
*Note: This is already reflected in Task 1, Step 5 of the plan.*

### 4.3 Test Helper Updates
The plan notes that "tests that call `test_state()` and expect no auth — they now need to include `Authorization: Bearer test-token`". This is a significant ripple effect. The plan accounts for this in Task 3 Step 8, but implementers should be prepared for a large number of test failures upon the initial state change.

## 5. Conclusion

**Verdict: APPROVED.**

The plan is ready for execution. It is technically sound, addresses all known regressions, and provides a clear path to a fully authenticated River Engine environment.
