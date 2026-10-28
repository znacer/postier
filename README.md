# PSTbeetle — PST Mail Explorer & RAG

Portable, local-first desktop mail explorer (3-pane web-standard layout built with **Dioxus 0.7 Desktop** and **Dioxus Components** / `dioxus-primitives`) for browsing Outlook PST archives, rendering HTML and plain-text emails, labeling messages with **Laya-GGUF** or an **LLM Judge**, and querying a collapsible/resizable RAG Chat pane backed by an OpenAI-compatible private LLM endpoint.

## Features

- **Modern 3-Pane Mail Explorer UI** (native Rust + **Dioxus 0.7 Desktop** + **Dioxus Components** in `src/components/` on Linux, macOS, and Windows):
  - **Left Folder, Mailbox & Label Sidebar**: Collapsible navigation sidebar (`285px` expanded / `42px` rail) built with `Sidebar*`, `Collapsible`, `ToggleGroup`, `Input`, `Checkbox`, `Switch`, `Badge`, and `ScrollArea` components. Supports bilingual UI switching (**Français** / **English**), PST archive and hierarchical folder trees (`SidebarMenu` / `SidebarMenuSub`), independent RAG retrieval checkboxes, label filter badges with live counts, Laya-GGUF model management, AI endpoint settings, and storage configuration.
  - **Center Message Table & Rich Email Reader**:
    - Action bar powered by `Toolbar`, `ToolbarGroup`, `ToolbarSeparator`, `ToggleGroup`, `Input`, and `Button` components, with a quick-filter bar (FTS5/BM25 with `/` keyboard shortcut), multi-select `Checkbox` rows, `Shift`-key range selection (`Shift + Click` or `Shift + ↑/↓`), and sortable columns (`Subject`, `From`, `Folder`, `Labels`, `Date`).
    - **Structured Envelope Header**: Displays sender `Avatar` / `AvatarFallback` initials, parsed `From` (`De`), `To` (`À`), and `Cc` (`Cc`) contact pills (display name + `<email@domain>`), localized human-readable date and time badges (`chrono`), folder path pill, attachment `Badge` chips, and manual/automatic label controls.
    - **Rich HTML & Plain-Text Email Rendering**: Sanitizes raw HTML emails (stripping scripts, trackers, unsafe attributes, and light-mode-only inline color overrides while preserving tables, lists, headings, blockquotes, and links) and upgrades plain-text emails into semantic HTML with structured reply/forward header cards, nested blockquotes, clickable URLs/emails, signatures, and legal disclaimer blocks inside a `ScrollArea`.
    - **Draggable Splitters**: Smoothly resize the message table height, mail header height, and chat pane width directly with the mouse.
  - **Right Collapsible & Resizable RAG Chat Pane**: Collapse into a `42px` rail or drag-resize the width (`310px` to window bounds). Includes suggested starter prompts, `Avatar`-badged message bubbles with Markdown-formatted assistant responses, expandable LLM context inspection (`Checkbox` + `Label`), and collapsible cited evidence cards (`Collapsible` + `ItemGroup` / `Item` components) with one-click `Open Mail` navigation.
- **Email Labeling (`Laya-GGUF` or `LLM as a Judge`)**:
  - **Laya Decision Model (`ggml-org/Laya-GGUF`)**: In-process pure-Rust GGUF v3 reader and ModernBERT + Laya Decision Head inference engine (`F32`, `F16`, `BF16`, `Q8_0` tensors, BPE tokenizer from `tokenizer.ggml.*` metadata, `[CLS] choice question: ... [SEP] [MASK] opt ... [SEP] state [SEP]` prompt formatting, and temperature scaling). Ships with a packaged in-process GGUF decision model that works offline out of the box, and can load or download full `Laya-bf16.gguf` / `Laya-Q8_0.gguf` weights from [`ggml-org/Laya-GGUF`](https://huggingface.co/ggml-org/Laya-GGUF).
  - **LLM as a Judge**: Structured JSON classification against user-defined label definitions (`name`, `description`/criteria, `color`) using the configured OpenAI-compatible endpoint.
  - Label a single email, a multi-checked selection, or batch-label the current filtered explorer view with background progress and cancellation support.
- **PST & Search Infrastructure**:
  - Pure-Rust read-only PST extraction (`outlook-pst`), sender & `To`/`Cc` recipient resolution from MAPI properties, multi-PST index management, bundled SQLite with FTS5/BM25, content-hash deduplication, optional remote email embeddings with hybrid retrieval, and an isolated mail-source interface.

For safe end-to-end development, the file picker also accepts `.jsonl` fixtures with one `Email` object per line.

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `↓` or `j` | Select next email in the table |
| `↑` or `k` | Select previous email in the table |
| `Shift + Click` | Select a contiguous range of emails between the anchor and clicked row |
| `Shift + ↓/↑` or `Shift + j/k` | Extend range selection to the next or previous email |
| `Space` or `x` | Toggle selection checkbox on the focused email |
| `/` | Focus the quick-filter search input |
| `Esc` | Clear active filter query or multi-email selection |

## Build and Run (`dx`)

### Prerequisites

1. **Rust** (Edition 2024 toolchain via `rustup`).
2. **Dioxus CLI (`dx`)**:
   ```shell
   curl -sSL https://dioxus.dev/install.sh | sh
   # or via cargo:
   cargo install dioxus-cli
   ```
3. **Platform WebView dependencies** (required by Dioxus Desktop / Wry):
   - **Linux (Debian/Ubuntu)**: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`
   - **Windows**: WebView2 runtime (pre-installed on Windows 10/11) and MSVC build tools
   - **macOS**: WebKit (included with macOS / Xcode Command Line Tools)

### Commands

```shell
# Run the desktop application in development mode (with hot-reloading)
dx serve --platform desktop

# Check code & formatting with the Dioxus CLI
dx check

# Build optimized release desktop binary
dx build --platform desktop --release

# Package / bundle the desktop application for distribution
dx bundle --platform desktop --release
```

When started in development mode (`dx serve`), debug builds print sanitized Rig request diagnostics and errors to the terminal. They include the HTTP method, endpoint path, model, authentication presence, and payload sizes, but never the bearer token, question, or email context.

The release profile enables LTO, stripping, size optimization, and abort-on-panic. SQLite is compiled into the executable (`rusqlite` bundled) and HTTP uses `rustls`, so no external database or TLS runtime is required.

Runtime data defaults to `data/` beside the executable. The storage directory can be changed in the UI and the selection is remembered in `storage-directory.txt`; the selected directory contains `index.sqlite`, `credentials.json`, and any downloaded `Laya-Q8_0.gguf` weights.

## Fixture Example

```json
{"message_id":"m1","conversation_id":"atlas","folder":"Inbox","subject":"Project Atlas","sender":"Alice Martin <alice@example.test>","recipients":"Bob Dupont <bob@example.test>\nCc: clara@example.test","sent_at":"2025-03-12T09:00:00Z","body":"The launch is postponed until June.","attachment_names":"plan.pdf"}
```

## Security Boundary

- BM25 retrieval, ranking, indexing, HTML email sanitization, and Laya-GGUF decision labeling run 100% locally.
- Remote embedding indexing and LLM-as-a-Judge labeling are opt-in and send bounded email plain text (with HTML tags and legal disclaimers stripped) to the configured OpenAI-compatible endpoint.
- The application requires HTTPS except for an endpoint whose host is exactly `127.0.0.1`; local loopback LLMs may use HTTP.
- In BM25-only mode, only the visible, character-bounded context plus the current question is sent.

See [`docs/architecture.md`](docs/architecture.md) for architecture details and [`CONTRIBUTING.md`](CONTRIBUTING.md) for development guidelines.
