# River Engine: Future Considerations

**Date:** 2026-03-16

This document captures features and improvements that were explicitly out of scope for the initial implementation but can now be considered for future development.

---

## 1. Discord Adapter Enhancements

These features were deferred from the initial Discord adapter implementation:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Embed support** | Rich message formatting with embeds (titles, fields, colors, images) | Medium |
| **File/attachment handling** | Upload and download files through Discord | Medium |
| **Voice channel support** | Join voice channels, text-to-speech, audio processing | High |
| **Multiple guild support** | Single adapter instance serving multiple Discord servers | Medium |
| **Message edit/delete events** | React to message edits and deletions | Low |

---

## 2. Orchestrator Enhancements

Features for more sophisticated model and resource management:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Request queuing** | Queue inference requests when resources are busy instead of failing | Medium |
| **Priority preemption** | Interactive requests can evict batch jobs to free resources | Medium |
| **Model preloading** | Predictive loading based on usage patterns and schedules | High |
| **Multi-node distribution** | Distribute models across multiple machines | High |
| **Prometheus metrics** | Export metrics for monitoring dashboards | Low |
| **Agent restart** | Detect unhealthy agents and trigger restart via systemd | Medium |
| **Priority queue** | Interactive > Scheduled > Background request ordering | Medium |
| **Persistence** | SQLite for historical data and crash recovery | Medium |

---

## 3. Platform & Distribution

Cross-platform and deployment improvements:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Docker images** | Official container images for easy deployment | Medium |
| **macOS support** | Native builds and testing on macOS | Medium |
| **Windows support** | Native builds and testing on Windows | High |
| **Flake support** | Nix flake packaging (currently standalone modules only) | Low |

---

## 4. Additional Adapters

The adapter interface is defined; these platforms could be added:

| Adapter | Description | Complexity |
|---------|-------------|------------|
| **Slack** | Slack workspace integration | Medium |
| **Matrix** | Matrix/Element chat integration | Medium |
| **IRC** | Internet Relay Chat integration | Low |
| **Telegram** | Telegram bot integration | Medium |
| **Web UI** | Browser-based chat interface | High |
| **CLI** | Interactive terminal adapter | Low |
| **Email** | IMAP/SMTP email integration | Medium |

---

## 5. Security Hardening

Security features deferred for initial implementation:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Encryption at rest** | Encrypt SQLite databases and state files | Medium |
| **TLS for internal comms** | HTTPS between gateway, orchestrator, adapters | Medium |
| **Authentication** | API keys or tokens for service communication | Medium |
| **Audit logging** | Detailed logs for security auditing | Low |

---

## 6. Advanced Agent Features

Higher-level agent capabilities:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Negotiated priority** | Agents can negotiate priority based on context | High |
| **Memory consolidation** | Algorithms to compress/summarize old memories | High |
| **Co-processor architecture** | Specialized sub-models for specific tasks | High |
| **Formal verification** | Verify agent behavior against specifications | Very High |
| **Onboarding flow** | Guided setup for new agents | Medium |

---

## 7. Publishing & Integration

External publishing and federation:

| Feature | Description | Complexity |
|---------|-------------|------------|
| **Tangled publishing** | Publish to Tangled network | Medium |
| **AT Protocol** | Bluesky/AT Protocol integration | High |
| **Webhooks** | Outbound webhooks for events | Low |
| **REST API clients** | Generated client libraries | Medium |

---

## Priority Recommendations

### Quick Wins (Low complexity, high value)
1. Message edit/delete events for Discord
2. Prometheus metrics for orchestrator
3. CLI adapter for testing
4. Webhooks for external integrations

### Medium-term (Moderate effort)
1. Request queuing in orchestrator
2. Embed support for Discord
3. Docker images
4. Slack adapter

### Long-term (Significant investment)
1. Multi-node distribution
2. Memory consolidation algorithms
3. Voice channel support
4. Web UI adapter

---

## Notes

- The adapter interface (`POST /incoming` + `POST /send`) is stable and documented
- New adapters can be added without modifying core components
- The orchestrator's model loading system is extensible for new features
- NixOS modules can be extended with additional options as features are added

Good luck with testing! The system is ready for real-world use.
