# river-protocol Fix Spec

> Synthesized from reviews dated 2026-04-03
> Priority: High

## Summary

river-protocol is a types-only crate with good code quality but critical gaps in testing and spec compliance. Two structs (Ground, Attachment) diverge from spec, and zero tests exist despite explicit spec requirement. Estimated effort: 1 day.

## Critical Issues

### Issue 1: Missing serde round-trip tests

- **Source:** Both reviews
- **Problem:** Spec explicitly requires "Unit tests for serde round-trips on all types, especially ProcessEntry with the new tagged enum format." Zero tests exist.
- **Fix:** Add comprehensive test module with round-trip tests for all types
- **Files:** `crates/river-protocol/src/lib.rs` (add `#[cfg(test)]` module) or `crates/river-protocol/tests/`
- **Tests:**
  - `side_serde_roundtrip`
  - `baton_serde_roundtrip`
  - `channel_serde_roundtrip`
  - `author_serde_roundtrip`
  - `attachment_serde_roundtrip`
  - `ground_serde_roundtrip`
  - `process_entry_worker_roundtrip`
  - `process_entry_adapter_roundtrip`
  - `process_entry_embed_roundtrip`
  - `registry_serde_roundtrip`
  - `model_config_roundtrip`
  - All registration types roundtrip
  - `process_entry_tagged_discrimination` (verify JSON has correct "type" field)

### Issue 2: Ground struct diverges from spec

- **Source:** Both reviews
- **Problem:** Spec defines `Ground { channel, adapter }` but implementation has `Ground { name, id, channel }`. Semantic change from "destination" to "person".
- **Fix:** Update spec to match implementation (implementation appears intentionally better - describes human operator)
- **Files:** `docs/superpowers/specs/2026-04-03-river-protocol-spec.md`
- **Tests:** Covered by serde roundtrip tests above

### Issue 3: Attachment struct diverges from spec

- **Source:** Both reviews
- **Problem:** Spec has optional `size`, implementation has required `size` and added `id` field
- **Fix:** Update spec to match implementation (id field needed for Discord attachments, size always available)
- **Files:** `docs/superpowers/specs/2026-04-03-river-protocol-spec.md`
- **Tests:** Covered by serde roundtrip tests above

## Important Issues

### Issue 4: API key exposed in Debug output

- **Source:** Both reviews
- **Problem:** `ModelConfig` derives `Debug`, which will print `api_key` in logs
- **Fix:** Implement `Debug` manually to redact api_key, or use a wrapper type
- **Files:** `crates/river-protocol/src/model.rs`
- **Tests:** Add test that Debug output doesn't contain actual key value

### Issue 5: serde rename_all inconsistency

- **Source:** Both reviews
- **Problem:** Spec uses `lowercase`, implementation uses `snake_case`. Functionally equivalent for current variants but inconsistent.
- **Fix:** Change to `lowercase` to match spec (or update spec to `snake_case`)
- **Files:** `crates/river-protocol/src/identity.rs`, `crates/river-protocol/src/registry.rs`
- **Tests:** Serde tests will verify correct serialization

## Minor Issues

### Issue 6: Missing PartialEq derives

- **Source:** First review
- **Problem:** Types like Attachment, Author, Ground, ModelConfig lack PartialEq, making testing harder
- **Fix:** Add `PartialEq` derive to all types that can support it
- **Files:** All source files in crate
- **Tests:** Enables assertion-based testing

### Issue 7: No Default implementations

- **Source:** Both reviews
- **Problem:** Only Registry has Default. Other types could benefit.
- **Fix:** Add Default where sensible (e.g., Author with empty strings)
- **Files:** `crates/river-protocol/src/identity.rs`
- **Tests:** Test default values

## Spec Updates Needed

1. Update `Ground` struct definition to match implementation (name, id, channel)
2. Update `Attachment` struct to include `id` field and make `size` required
3. Document rationale for these changes

## Verification Checklist

- [ ] All serde round-trip tests pass
- [ ] ProcessEntry tagged enum discrimination verified
- [ ] API key redacted from Debug output
- [ ] serde rename_all consistent with spec
- [ ] PartialEq added to relevant types
- [ ] Spec updated for Ground and Attachment
