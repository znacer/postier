# Contributing to PoSTier (`pst-rag`)

Thank you for your interest in contributing to **PoSTier**! This document outlines the project structure, setup instructions, coding conventions, and Dioxus (`dx`) development workflow.

---

## 1. Development Environment Setup

### Rust Toolchain & Dioxus CLI (`dx`)
PoSTier uses **Rust Edition 2024** and the **Dioxus CLI (`dx`)** for serving, checking, building, and bundling the desktop app:

```shell
rustup update stable
rustup component add rustfmt clippy

# Install Dioxus CLI (dx)
curl -sSL https://dioxus.dev/install.sh | sh
# or: cargo install dioxus-cli
```

### System Dependencies (Dioxus Desktop / Wry)
Because the desktop UI is powered by `dioxus` (`0.7`, `desktop` feature) and `wry`, you need your platform's WebView development headers:

- **Ubuntu / Debian**:
  ```shell
  sudo apt-get update
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    patchelf \
    build-essential \
    curl \
    wget \
    file
  ```
- **Fedora / RHEL**:
  ```shell
  sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel patchelf
  ```
- **Windows**: Install [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (Desktop development with C++); WebView2 is pre-installed on Windows 10/11.
- **macOS**: Install Xcode Command Line Tools (`xcode-select --install`).

---

## 2. Project Architecture

```text
config.toml                 # Compile-time shipped defaults ([llm], [agent], [laya], [ui])
assets/
├── logo.svg                # PoSTier vector logo
├── logo.png                # PoSTier high-resolution app icon
├── logo.ico                # Windows desktop application icon
└── dx-components-theme.css # Base theme variables for Dioxus Components
src/
├── main.rs                 # Application entry point
├── model.rs                # Shared domain models (Email, EmailRecord, SearchHit, LabelDefinition, etc.)
├── pst.rs                  # Streaming Outlook PST archive & JSONL fixture reader (MAPI sender, To/Cc/Bcc, body)
├── email_html.rs           # HTML5 email sanitizer (html5ever), plain-text-to-HTML formatter, participant & date parsers
├── text.rs                 # Plain-text indexing, EN/FR Snowball stemming, and best-passage snippet extraction
├── db.rs                   # SQLite + FTS5/BM25 schema, migrations, and message/source persistence
├── db/
│   ├── query.rs            # Structured query parser, FTS5/BM25 search, thread collapsing, and email_stats
│   ├── labels.rs           # Label definitions and message-label assignment persistence
│   └── search.rs           # Optional vector embedding storage and reciprocal-rank hybrid search
├── laya.rs                 # Pure-Rust GGUF v3 reader & ModernBERT + Laya Decision Head inference engine
├── laya/
│   ├── gguf.rs             # GGUF v3 binary parser and tensor dequantization
│   ├── ops.rs              # Tensor math primitives (matmul, RoPE, GeGLU, layer norm, attention)
│   ├── packaged.rs         # Built-in offline packaged GGUF decision model generator
│   └── tokenizer.rs        # BPE tokenizer loaded from GGUF metadata
├── llm.rs                  # OpenAI-compatible client (Rig), TOML config loader, LLM Judge, and endpoint validation
├── llm/
│   ├── agent.rs            # Tool-calling research agent (search_emails, read_email, get_thread, email_stats)
│   ├── budget.rs           # Adaptive token budgeting and Rig history trimming hooks
│   └── context.rs          # Bilingual agent prompts, email body cleaner, citation helpers, and context builder
├── i18n.rs                 # French and English UI translations and agent status strings
├── components/             # Styled Dioxus UI component library (dioxus-primitives + dioxus-icons)
│   ├── mod.rs              # Exports Avatar, Badge, Button, Checkbox, Collapsible, DropdownMenu, Input, Item,
│   │                       # Label, ScrollArea, Separator, Sheet, Sidebar, Skeleton, Switch, ToggleGroup, Toolbar, Tooltip
│   └── <component>/        # Component implementation (component.rs) and scoped CSS module (style.css)
├── app.rs                  # Dioxus 0.7 desktop root component, state management, resize & worker tasks
├── app/
│   ├── actions.rs          # State transitions, async workers, file pickers, and DB queries
│   ├── actions/
│   │   └── labeling.rs     # Single, selection, and batch email labeling workers (Laya & LLM Judge)
│   ├── theme.rs            # Global CSS design system (DX_COMPONENTS_THEME_CSS + APP_CSS), palette overrides,
│   │                       # splitters, and reusable component wrappers
│   ├── sidebar.rs          # Left collapsible sidebar (Sidebar*, Collapsible, ToggleGroup, Checkbox, Switch, Input)
│   ├── explorer.rs         # Center mail table, Toolbar, structured envelope header (Avatar, Badge), and reader
│   ├── chat.rs             # Right RAG chat pane, Avatar bubbles, mail:// links, and Collapsible evidence cards
│   ├── markdown.rs         # Markdown block renderer for assistant responses
│   └── markdown/
│       └── parser.rs       # Lightweight Markdown block and inline link parser
└── tests/                  # Unit and integration tests by module (db, query, llm, laya, text, email_html, i18n, etc.)
```

---

## 3. Running & Building with `dx`

Use the `dx` CLI during development and packaging:

```shell
# Launch the desktop app in development mode with hot-reloading
dx serve --platform desktop

# Validate RSX components and Dioxus rules
dx check

# Format RSX macros in source files
dx fmt --check

# Build release binary for desktop
dx build --platform desktop --release

# Bundle native desktop installers/packages
dx bundle --platform desktop --release
```

---

## 4. Coding Guidelines

1. **Security & Privacy First**:
   - **Never log sensitive data**: Do not print bearer tokens, user questions, or raw email bodies in logs or error messages.
   - **Strict HTTPS enforcement**: Remote LLM/embedding calls must require `https://` except when targeting `http://127.0.0.1` for local inference servers.
   - **Safe HTML rendering**: Any HTML rendered in the email reader (`dangerous_inner_html`) must pass through `email_html::format_email_body_html` / `sanitize_html_email`. Never allow `<script>`, `<iframe>`, `<object>`, inline `on*` event attributes, `javascript:` URLs, or external tracking pixels.

2. **Bilingual Support (`Français` & `English`)**:
   - All user-facing strings belong in `src/i18n.rs` on the `Language` enum (and bilingual agent prompts in `src/llm/context.rs`).
   - When adding a new UI label, button, or status message, provide both `Language::French` and `Language::English` translations and update `src/tests/i18n.rs`.

3. **UI Components & Styling (`Dioxus 0.7` + `src/components/`)**:
   - Build UI views using the styled components in `src/components/` (`Button`, `Badge`, `Avatar`, `Checkbox`, `Collapsible`, `Input`, `Item`, `Label`, `ScrollArea`, `Separator`, `Sidebar*`, `Switch`, `ToggleGroup`, `Toolbar`, etc.) rather than raw unstyled HTML primitives where a component exists.
   - Global theme tokens and dark-palette overrides for `--primary-color-*`, `--secondary-color-*`, and `--bg-*` are defined in `src/app/theme.rs` (`DX_COMPONENTS_THEME_CSS` and `APP_CSS`).
   - Keep long-running operations (PST indexing, Laya inference, LLM agent queries) off the UI thread by dispatching `AppTask::Worker` tasks and reporting progress via `WorkerEvent::Progress`.

4. **Preserve Documentation**:
   - Keep existing doc comments and module documentation accurate when refactoring or extending APIs.

---

## 5. Verification & Testing

Before opening a pull request, run the verification checks locally:

```shell
# 1. Validate Dioxus RSX and formatting
dx check
dx fmt --check

# 2. Run the test suite
cargo test

# 3. Build the desktop target with dx
dx build --platform desktop
```

### Adding Tests
- Place unit tests in the corresponding file under `src/tests/` (e.g., `src/tests/email_html.rs`, `src/tests/db.rs`, `src/tests/query.rs`, `src/tests/llm.rs`, `src/tests/i18n.rs`) or component/view tests in `src/app/theme.rs`.
- Use `.jsonl` fixtures or in-memory/temporary SQLite databases rather than committing real or personal `.pst` archives.

---

## 6. Pull Request Process

1. Create a descriptive branch from `main` (or the active feature branch).
2. Keep commits focused and atomic.
3. Ensure `dx check`, `dx fmt --check`, and `cargo test` pass without errors.
4. Describe what changed, why it changed, and how it was tested in your PR description.
