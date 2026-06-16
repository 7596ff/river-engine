# Attachments — Design (v1)

Status: approved 2026-06-15. Spec for the first attachments card. The
wall (ch. 05, ch. 06) is silent on attachments; this document is the
design decision that fills that silence, and a summary entry will be
added to `docs/decisions.md` when the implementation lands.

## Scope

- Full attachment model: the `Adapter` trait and the channel JSONL shape
  both learn about attachments. New `Feature` variants gate per-adapter
  support so the system prompt tells the agent honestly which channels
  carry files.
- **v1 ships discord inbound and outbound.** The local surface declares
  neither feature for v1. Other adapters that don't declare the
  features carry no attachments and the channel-log entries from them
  simply lack the field — additive on both axes.

## Channel-log entry shape

A new optional `attachments` field on both `role:other` and `role:agent`
entries. Missing field = no attachments; existing logs read unchanged.

```json
{"id":"01JX...","role":"other","author":"cassie","author_id":"123",
 "content":"check this out","adapter":"discord","msg_id":"456",
 "attachments":[
   {"filename":"cat.png","path":"attachments/01JX.../cat.png",
    "mime":"image/png","size":412034}
 ]}
```

Per-attachment fields:

- `filename` — the platform's original name, sanitized (path separators
  and control chars stripped). For inbound, this preserves what the
  sender called the file even when `path` had to be renamed for
  collision avoidance.
- `path` — workspace-relative path to the file, or `null` if the
  attachment could not be stored (see *skipped* below). The workspace
  is ground truth (wall ch. 02), so relative paths anchor against it.
- `mime` — adapter's best-effort MIME type.
- `size` — bytes; the file's true size if stored, or the adapter's
  declared size if skipped.
- `skipped` (optional) — present only when `path` is `null`. One of
  `"too_large"`, `"download_failed"`. The agent learns that something
  existed even when it can't be opened.

## Storage layout

Inbound files land under:

```
{workspace}/attachments/{entry_ulid}/{sanitized_filename}
```

- Flat by ULID, matching ch. 05's flat-channel-namespace spirit.
- One directory per channel-log entry. Multiple attachments on one
  message share a directory.
- Collisions within a single ULID directory (rare; Discord normally
  prevents same-name within a message) get `-2`, `-3` suffixes added
  before the extension.
- Lives under the workspace's indexed roots from ch. 02, so the sync
  service hashes, embeds (where the type supports it), and joins
  attachments to the activation graph like any other workspace file.

Outbound attachments are **not copied**. The agent supplies
workspace-relative paths it has already written; the entry records
those paths directly. Two truths are not created.

## Inbound flow (discord)

Extends ch. 05's write-then-notify:

```
discord MessageCreate received
  → engine generates the entry ULID
  → for each attachment in msg.attachments:
      download from CDN URL with one retry on transient failure
        → on success: write to {workspace}/attachments/{ulid}/{filename}
        → on failure: record { path: null, skipped: "download_failed" }
        → over max_bytes:  record { path: null, skipped: "too_large" }
  → build entry { content, attachments: [...] }
  → APPEND entry to channel log
  → push notification {channel, ulid}
```

Binding ordering rule: **every attachment with a non-null `path` is
durably written to disk before the JSONL line is appended.** A torn
turn never leaves a log entry pointing at a missing file. Failed and
oversized attachments are recorded as skipped — text content is never
lost over a broken blob.

Retry policy: at most one in-process retry per download, with a short
backoff. Discord CDN URLs are signed and may already be expiring; a
background queue would race the expiry. After the retry, we accept
the failure.

Filename sanitization: strip path separators (`/`, `\`), null bytes,
and control characters. Empty results after sanitization use the
adapter's attachment id as the filename.

## Outbound flow (discord)

The `speak` tool gains an optional `attachments` parameter — a list of
workspace-relative paths.

```
agent calls speak { channel, content, attachments: ["notes/diagram.png"] }
  → channel layer validates each path:
       - relative, no "..", no absolute paths
       - resolves inside the workspace after symlink resolution
       - file exists and is readable
  → outbound request → discord adapter
  → adapter reads each file, uploads as multipart to discord
  → discord returns msg_id
  → engine appends role:agent entry { content, attachments: [...] }
       using the workspace-relative paths the agent supplied
```

For each outbound attachment the engine fills `filename` from the
supplied path's leaf, `size` by stat'ing the file, and `mime` by
extension lookup (falling back to `application/octet-stream`).

Notable rules:

- **Validation at the channel layer**, not the adapter — the same rules
  apply to any future adapter declaring `AttachmentsSend`.
- **No engine-side size cap** on outbound. Whatever Discord rejects
  surfaces as a normal tool error to the agent.
- **Feature gating.** A `speak` call with attachments to a channel
  whose adapter does not declare `AttachmentsSend` returns a tool
  error before any platform call.

## Adapter trait additions

Two new `Feature` enum variants:

- `AttachmentsReceive` — adapter forwards inbound attachments to the
  channel layer.
- `AttachmentsSend` — adapter accepts attachments on outbound requests.

Discord declares both. The local surface declares neither in v1. Per
ch. 06, feature declarations fold into the agent's system prompt so the
model knows each channel's real capabilities.

## Config

Optional `attachments` block on the agent's config entry (ch. 09
conventions):

```json
"attachments": {
  "max_bytes": 26214400,
  "download_timeout_secs": 30
}
```

- `max_bytes` — per-file cap on inbound downloads. Default 25 MiB
  (Discord's free-tier upload limit). Over-cap files are recorded as
  `skipped: "too_large"`.
- `download_timeout_secs` — per-attempt HTTP timeout. Default 30.
- Omitted block = defaults. Unknown keys rejected (`deny_unknown_fields`,
  matching the rest of the config).

## Model perception

Choice: **metadata only**. When the turn loop renders a channel entry
into the model's context, attachments appear as a metadata line
alongside the message text — filename, mime, size, workspace path (or
the skipped reason). The agent decides whether to open the file using
its existing file tools, which handle images multimodally where the
model supports it. No special multimodal embedding in context
assembly; no per-model capability table at the channel layer.

## Memory integration

No new memory rules. Attachments are workspace files; workspace files
are how memory works (wall ch. 02):

- Downloaded attachments live under the indexed roots, so the sync
  service hashes them, embeds those it can, and joins them to the
  activation graph as ordinary nodes.
- The agent's file-tool reads bump activation per ch. 02 — opening
  `cat.png` warms it and its neighbors. No special-case code path.
- Outbound attachments are already in the workspace by the time
  `speak` is called; they are already indexed.

## Failure modes & contracts

- **Write-then-append for inbound.** Every attachment with a non-null
  `path` is on disk before the JSONL entry is appended. Channel-log
  entries never point at missing files.
- **Per-attachment status, never drop the entry.** Failed downloads
  and oversized files append with `path: null` and a `skipped` reason;
  text is preserved.
- **One in-process retry on download.** No background retry queue.
- **Filename sanitization** strips separators, null bytes, and
  control characters. In-ULID-dir collisions get `-N` suffixes.
- **Outbound path validation** — workspace-relative only, no `..`, no
  symlink escape. Validation happens in the channel layer.
- **No outbound copy.** Outbound entries reference workspace paths
  directly; the engine does not duplicate files into `attachments/`.
- **Feature gating.** Attachments on `speak` to a non-supporting
  channel = tool error before any platform call.
- **Adapter additivity.** Adapters that declare neither feature
  carry no attachments; the channel-log entry shape simply omits the
  field. Existing logs read unchanged.

## Out of scope (v1)

- **Local surface attachments** — no inbound, no outbound. Future card
  decides the upload mechanism (multipart over HTTP, websocket binary
  frames, or pre-staging to `attachments/`).
- **Outbound size caps** — Discord's own limits suffice; revisit if
  the trust model changes.
- **Discord embeds, stickers, voice messages, polls.** Only files in
  `msg.attachments` are ingested. Other rich content is silently
  dropped, as it is today.
- **Content-addressed storage / dedup.** Two copies of the same file
  occupy two entries. Add later if it ever matters.
