# PST RAG

Portable, local-first Windows desktop prototype for searching Outlook archives and sending only selected evidence to an OpenAI-compatible private LLM endpoint.

## Current state

Implemented: native Rust/Iced UI on Linux and Windows, pure-Rust read-only PST extraction, multi-PST index management with reversible archive/folder retrieval filters and per-PST deletion, bundled SQLite with FTS5/BM25, content-hash deduplication, optional remote email embeddings with hybrid retrieval, standalone local search, bounded context preview, a Rig RAG agent using an OpenAI-compatible private LLM endpoint, optional HTTPS-only private LLM calls with a masked API-key input, evidence display, and an isolated mail-source interface.

PST traversal uses Microsoft's cross-platform `outlook-pst` implementation and extracts folders, messages, subjects, senders, recipients, timestamps, and searchable bodies. This now works without Outlook on Linux and Windows. Corpus validation is still required before claiming production-grade compatibility; libpff remains a fallback if damaged or legacy archives expose gaps. VSS is not attempted on Linux and is not silently attempted on Windows: creating shadow copies commonly requires privileges unavailable to a portable non-admin app.

For safe end-to-end development, the file picker accepts JSONL fixtures with one `Email` object per line. This is not a deployment feature.

## Build and test

```shell
cargo test
cargo build --release
cargo build --release --target x86_64-pc-windows-msvc
```

When started with `cargo run`, debug builds print sanitized Rig request diagnostics and errors to the terminal. They include the HTTP method, endpoint path, model, authentication presence, and payload sizes, but never the bearer token, question, or email context.

The release profile enables LTO, stripping, size optimization, and abort-on-panic. SQLite is compiled into the executable and HTTP uses rustls, so no database or TLS runtime is installed. Cross-compilation still requires the Microsoft target/linker; the canonical release should be produced on a pinned Windows CI runner.

Runtime data defaults to `data/` beside the executable. The storage directory can be changed in the UI and the selection is remembered in `storage-directory.txt`; the selected directory contains both `index.sqlite` and `credentials.json`. The credentials file is restricted to the current user on Unix, but remains a local plaintext secret, so select a private directory and never place it in source control. A platform credential-vault implementation is still recommended for production rollout.

Each indexed PST appears in the sidebar. Its top-level checkbox includes or excludes the whole archive from subsequent retrieval, and its folder checkboxes narrow retrieval without deleting data or requiring reindexing. “Remove index” asks for confirmation and then deletes that PST's indexed messages and associated embeddings; it never modifies the original PST file.

## Fixture example

```json
{"message_id":"m1","conversation_id":"atlas","folder":"Inbox","subject":"Project Atlas","sender":"john@example.test","recipients":"team@example.test","sent_at":"2025-03-12T09:00:00Z","body":"The launch is postponed until June.","attachment_names":"plan.pdf"}
```

## Security boundary

- BM25 retrieval, ranking, and indexing remain local. Remote embedding indexing is opt-in and sends bounded email text to the configured OpenAI-compatible `/embeddings` endpoint.
- The application requires HTTPS except for an endpoint whose host is exactly `127.0.0.1`; local loopback LLMs may use HTTP.
- In BM25-only mode, only the visible, character-bounded context plus the current question is sent. In remote-embedding mode, bounded email text is sent during indexing and the question is sent once for hybrid retrieval.
- There is no telemetry, updater, public service, vector database, or local model.

See `docs/architecture.md` for the hardening and parser-validation gates.
