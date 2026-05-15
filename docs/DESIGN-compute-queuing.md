# Compute Queuing System Design

*Created: 2026-05-15*

## Problem Statement
Currently, `river-orchestrator` manages the lifecycle of local `llama-server` instances to optimize VRAM and compute resources. When an agent requests a model via `/model/request`, the orchestrator ensures it is loaded and returns the endpoint. However, if multiple agents request the same model, they are given the same endpoint.

Because `llama-server` relies heavily on KV cache management to maintain context across turns, simultaneous requests from different agents "mash" their contexts together. The server attempts to process disparate prompts concurrently, leading to degraded generation quality, hallucinated cross-contamination of contexts, and potential crashes or evictions of the active context slots. 

We need a system to ensure that an agent has **exclusive access** to a stateful `llama-server` process for the duration of a generation turn.

## Proposed Solution: Lease-Based Queuing

The orchestrator will implement a lease-based queuing system using tokio Semaphores. Instead of simply requesting an endpoint, agents will "acquire a lease" on a model.

### 1. State Changes (`OrchestratorState`)
- **Compute Locks:** Each `LocalModelEntry` will be augmented with a `tokio::sync::Semaphore` initialized with `1` permit.
- **Lease Tracking:** The orchestrator will maintain a map of active leases (`lease_id` -> `LeaseInfo`), where `LeaseInfo` tracks the `model_id`, `agent_name`, and `acquired_at` timestamp.

### 2. New API Endpoints
We will introduce two new endpoints to replace the usage of `/model/request` for local models:

#### `POST /model/acquire`
- **Request:** `{"model": "model_id", "agent": "agent_name", "timeout_seconds": 120}`
- **Behavior:**
  1. The orchestrator attempts to acquire a permit from the model's semaphore, blocking up to `timeout_seconds`.
  2. If the timeout is reached while waiting in the queue, it returns a `408 Request Timeout` (or a specific "Queue Timeout" status).
  3. Once the permit is acquired, it ensures the model is loaded (reusing existing spawn/discovery logic).
  4. If the model is ready, it generates a unique `lease_id`, stores it, "forgets" the tokio permit (so it isn't dropped at the end of the request handler), and returns the lease to the agent.
- **Response:** `{"status": "ready", "lease_id": "...", "endpoint": "..."}`

#### `POST /model/release_lease`
- **Request:** `{"lease_id": "..."}`
- **Behavior:**
  1. Looks up the lease. If it exists, removes it from the tracking map.
  2. Identifies the associated model and adds `1` permit back to its semaphore, effectively passing control to the next waiting request.
- **Response:** `{"acknowledged": true}`

### 3. Agent Lifecycle Changes
The `AgentTask` in the gateway must be updated to use this new lifecycle:
1. **Before Turn:** Call `/model/acquire`. If it times out, wait and retry.
2. **During Turn:** Execute the Think/Act loop using the provided endpoint. The agent knows it has exclusive access to the KV cache.
3. **After Turn:** (In the Settle phase), call `/model/release_lease`. 

### 4. Safety Mechanisms
- **Lease Expiration/Watchdog:** To prevent a crashed agent from holding a lock forever, the orchestrator needs a background watchdog task that periodically checks active leases. If a lease exceeds a maximum duration (e.g., 5 minutes), the orchestrator will forcibly revoke it, add a permit back to the semaphore, and optionally restart the underlying `llama-server` process to clear any stuck state.
- **Graceful Shutdown:** If the orchestrator shuts down, all leases are implicitly invalidated.

## Edge Cases and Considerations
- **External Models:** External APIs (like OpenAI or Anthropic) are stateless from the orchestrator's perspective. The `/model/acquire` endpoint should immediately return a "dummy" lease for external models without blocking on a semaphore.
- **Batch Processing vs Interactive:** We might later want to expand the semaphore to support >1 permit if the underlying `llama-server` is configured to handle multiple slots (`--parallel N`). For now, `1` permit enforces strict sequential isolation.