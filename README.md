# PST Mail Explorer & RAG

Portable, local-first desktop mail explorer (Thunderbird-style 3-pane layout) for browsing Outlook PST archives, labeling emails with **Laya-GGUF** or an **LLM Judge**, and querying a collapsible/extendable RAG Chat pane backed by an OpenAI-compatible private LLM endpoint.

## Current state

Implemented:
- **Thunderbird-style Mail Explorer UI** (native Rust/Iced on Linux and Windows):
  - **Left Folder & Label Tree**: Browse all messages, individual PST archives, folder hierarchies, or filter by assigned labels / unlabeled emails. Folder and archive checkboxes remain independent so you can browse any folder while controlling which archives/folders participate in RAG retrieval.
  - **Center Message Table & Reading Pane**: Quick-filter bar (FTS5/BM25), sortable columns (`Subject`, `Correspondents`, `Folder`, `Labels`, `Date`), and a lower reading pane displaying full headers, attachments, label badges with confidence scores, manual label toggle chips, and one-click `Label (Laya)`, `Label (LLM Judge)`, and `Ask in Chat` actions.
  - **Right Collapsible & Extendable Chat Pane**: Collapse into a `32px` vertical rail (`◀`/`▶`) or expand and resize width (`310px` to `760px`) via `◀ Extend`, `Shrink ▶`, or the width slider. Evidence cards include an `Open Mail` button to jump directly to the cited email in the explorer.
- **Email Labeling (`Laya-GGUF` or `LLM as a Judge`)**:
  - **Laya Decision Model (`ggml-org/Laya-GGUF`)**: In-process pure-Rust GGUF v3 reader and ModernBERT + Laya Decision Head inference engine (`F32`, `F16`, `BF16`, `Q8_0` tensors, BPE tokenizer from `tokenizer.ggml.*` metadata, `[CLS] choice question: ... [SEP] [MASK] opt ... [SEP] state [SEP]` prompt formatting, and temperature scaling). Ships with a packaged in-process GGUF decision model that works offline out of the box, and can load or download full `Laya-bf16.gguf` / `Laya-Q8_0.gguf` weights from [`ggml-org/Laya-GGUF`](https://huggingface.co/ggml-org/Laya-GGUF).
  - **LLM as a Judge**: Structured JSON classification against user-defined label definitions (`name`, `description`/criteria, `color`) using the configured OpenAI-compatible endpoint.
  - Label a single email (`Label Mail`) or batch-label the current filtered explorer view (`Label View`).
- **PST & Search Infrastructure**:
  - Pure-Rust read-only PST extraction (`outlook-pst`), multi-PST index management, bundled SQLite with FTS5/BM25, content-hash deduplication, optional remote email embeddings with hybrid retrieval, and an isolated mail-source interface.

For safe end-to-end development, the file picker accepts JSONL fixtures with one `Email` object per line. This is not a deployment feature.

## Build and test

```shell
cargo test
cargo build --release
cargo build --release --target x86_64-pc-windows-msvc
```

When started with `cargo run`, debug builds print sanitized Rig request diagnostics and errors to the terminal. They include the HTTP method, endpoint path, model, authentication presence, and payload sizes, but never the bearer token, question, or email context.

The release profile enables LTO, stripping, size optimization, and abort-on-panic. SQLite is compiled into the executable and HTTP uses rustls, so no database or TLS runtime is installed. Cross-compilation still requires the Microsoft target/linker; the canonical release should be produced on a pinned Windows CI runner.

Runtime data defaults to `data/` beside the executable. The storage directory can be changed in the UI and the selection is remembered in `storage-directory.txt`; the selected directory contains `index.sqlite`, `credentials.json`, and any downloaded `Laya-Q8_0.gguf` weights.

## Fixture example

```json
{"message_id":"m1","conversation_id":"atlas","folder":"Inbox","subject":"Project Atlas","sender":"john@example.test","recipients":"team@example.test","sent_at":"2025-03-12T09:00:00Z","body":"The launch is postponed until June.","attachment_names":"plan.pdf"}
```

## Security boundary

- BM25 retrieval, ranking, indexing, and Laya-GGUF decision labeling remain local. Remote embedding indexing and LLM-as-a-Judge labeling are opt-in and send bounded email text to the configured OpenAI-compatible endpoint.
- The application requires HTTPS except for an endpoint whose host is exactly `127.0.0.1`; local loopback LLMs may use HTTP.
- In BM25-only mode, only the visible, character-bounded context plus the current question is sent.

See `docs/architecture.md` for the hardening and parser-validation gates.
