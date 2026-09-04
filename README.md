# PST RAG

Portable, local-first Windows desktop prototype for searching Outlook archives and sending only selected evidence to an OpenAI-compatible private LLM endpoint.

## Current state

Implemented: native Rust/egui UI on Linux and Windows, pure-Rust read-only PST extraction, bundled SQLite with FTS5/BM25, content-hash deduplication, standalone local search, bounded context preview, optional HTTPS-only private LLM calls with a masked API-key input, evidence display, and an isolated mail-source interface.

PST traversal uses Microsoft's cross-platform `outlook-pst` implementation and extracts folders, messages, subjects, senders, recipients, timestamps, and searchable bodies. This now works without Outlook on Linux and Windows. Corpus validation is still required before claiming production-grade compatibility; libpff remains a fallback if damaged or legacy archives expose gaps. VSS is not attempted on Linux and is not silently attempted on Windows: creating shadow copies commonly requires privileges unavailable to a portable non-admin app.

For safe end-to-end development, the file picker accepts JSONL fixtures with one `Email` object per line. This is not a deployment feature.

## Build and test

```shell
cargo test
cargo build --release
cargo build --release --target x86_64-pc-windows-msvc
```

The release profile enables LTO, stripping, size optimization, and abort-on-panic. SQLite is compiled into the executable and HTTP uses rustls, so no database or TLS runtime is installed. Cross-compilation still requires the Microsoft target/linker; the canonical release should be produced on a pinned Windows CI runner.

Runtime data is created in `data/` beside the executable. Do not place bearer tokens in source control; a Windows Credential Manager-backed settings implementation is required before production rollout.

## Fixture example

```json
{"message_id":"m1","conversation_id":"atlas","folder":"Inbox","subject":"Project Atlas","sender":"john@example.test","recipients":"team@example.test","sent_at":"2025-03-12T09:00:00Z","body":"The launch is postponed until June.","attachment_names":"plan.pdf"}
```

## Security boundary

- Retrieval, ranking, indexing, archive content, and the complete query corpus remain local.
- The application requires HTTPS except for an endpoint whose host is exactly `127.0.0.1`; local loopback LLMs may use HTTP.
- Only the visible, character-bounded context plus the current question is sent.
- There is no telemetry, updater, public service, vector database, or local model.

See `docs/architecture.md` for the hardening and parser-validation gates.
