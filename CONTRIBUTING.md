# Contributing to PSTbeetle (`pst-rag`)

Thank you for your interest in contributing to **PSTbeetle**! This document outlines the project structure, setup instructions, coding conventions, and Dioxus (`dx`) development workflow.

---

## 1. Development Environment Setup

### Rust Toolchain & Dioxus CLI (`dx`)
PSTbeetle uses **Rust Edition 2024** and the **Dioxus CLI (`dx`)** for serving, checking, building, and bundling the desktop app:

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
src/
├── main.rs            # Application entry point
├── model.rs           # Shared domain models (Email, EmailRecord, SearchHit, LabelDefinition, etc.)
├── pst.rs             # Outlook PST archive & JSONL fixture reader (MAPI sender, To/Cc recipients, body)
├── db.rs              # SQLite + FTS5/BM25 index, vector embeddings store, and label persistence
├── laya.rs            # Pure-Rust GGUF v3 reader & ModernBERT + Laya Decision Head inference engine
├── llm.rs             # OpenAI-compatible RAG client (Rig), LLM-as-a-Judge labeler, and body cleaner
├── i18n.rs            # French and English UI translations and system prompts
├── app.rs             # Dioxus 0.7 desktop root component, state management, resize & worker tasks
├── app/
│   ├── actions.rs     # State transitions, async workers, file pickers, and DB queries
│   ├── theme.rs       # Global CSS design system (APP_CSS), colors, splitters, and reusable UI widgets
│   ├── sidebar.rs     # Left collapsible sidebar (mailboxes, folder tree, labels, Laya/AI/storage settings)
│   ├── explorer.rs    # Center mail table, toolbar, structured envelope header, and reading pane
│   ├── email_html.rs  # HTML email sanitizer, plain-text-to-HTML formatter, participant & date parsers
│   ├── chat.rs        # Right RAG chat pane, message bubbles, and cited source cards
│   └── markdown.rs    # Lightweight Markdown parser for assistant responses
└── tests/             # Unit and integration tests by module
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
   - All user-facing strings belong in `src/i18n.rs` on the `Language` enum.
   - When adding a new UI label, button, or status message, provide both `Language::French` and `Language::English` translations and update `src/tests/i18n.rs`.

3. **UI & Styling (`Dioxus 0.7`)**:
   - Prefer shared CSS classes and CSS custom properties (`--bg-surface`, `--bg-elevated`, `--text-primary`, `--border-default`, etc.) defined in `src/app/theme.rs` (`APP_CSS`).
   - Keep long-running operations (PST indexing, Laya inference, LLM queries) off the UI thread by dispatching `AppTask::Worker` tasks and reporting progress via `WorkerEvent::Progress`.

4. **Preserve Documentation**:
   - Keep existing doc comments and module documentation accurate when refactoring or extending APIs.

---

## 5. Verification & Testing

Before opening a pull request, run the verification checks locally:

```shell
# 1. Validate Dioxus RSX and formatting
dx check
dx fmt --check

# 2. Build the desktop target with dx
dx build --platform desktop
```

### Adding Tests
- Place unit tests in the corresponding file under `src/tests/` (e.g., `src/tests/email_html.rs`, `src/tests/db.rs`, `src/tests/llm.rs`, `src/tests/i18n.rs`).
- Use `.jsonl` fixtures or in-memory/temporary SQLite databases rather than committing real or personal `.pst` archives.

---

## 6. Pull Request Process

1. Create a descriptive branch from `main` (or the active feature branch).
2. Keep commits focused and atomic.
3. Ensure `dx check`, `dx fmt --check`, and the test suite pass without errors.
4. Describe what changed, why it changed, and how it was tested in your PR description.
