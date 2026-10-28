# Architecture and release gates

`MailSource -> normalized Email -> SQLite messages + FTS5 (always) -> optional remote embeddings -> BM25 or hybrid candidates -> bounded context -> Rig agent -> private LLM`

## UI Layer (`Dioxus 0.7 Desktop` + `src/components/`)

The desktop interface is organized as a 3-pane workspace rendered via Dioxus 0.7 Desktop and styled primitives from `src/components/` (`dioxus-primitives` + `dioxus-icons` with CSS modules and theme variables in `assets/dx-components-theme.css` / `src/app/theme.rs`):

- **Left Sidebar (`src/app/sidebar.rs`)**: Structured with `SidebarHeader`, `SidebarContent`, `SidebarGroup`, `SidebarGroupContent`, `SidebarMenu`, `SidebarMenuItem`, `SidebarMenuBadge`, `SidebarMenuSub`, and `SidebarMenuSubItem` inside a `ScrollArea`. Collapsible sections (`Mailboxes`, `Labels`, and `Settings` — collapsed by default and grouping `LLM Judge and RAG`, `Laya Decision Model`, and `Storage`) use `Collapsible`, `CollapsibleTrigger`, and `CollapsibleContent`. Form controls and filters use `ToggleGroup` / `ToggleItem`, `Checkbox`, `Switch`, `Input`, `Label`, `Badge`, `Separator`, and `Button`.
- **Center Explorer & Reading Pane (`src/app/explorer.rs`)**: Uses `Toolbar`, `ToolbarGroup`, `ToolbarSeparator`, `ToggleGroup`, `Input`, and `Button` for quick-filtering and batch/focused labeling controls. The message table and reading pane use `Checkbox`, `Avatar` / `AvatarFallback`, `Badge`, `Separator`, and `ScrollArea` separated by draggable horizontal splitters.
- **Right RAG Chat Pane (`src/app/chat.rs`, `src/app/markdown.rs`)**: Uses `ScrollArea`, `Avatar` / `AvatarFallback`, `Input`, `Checkbox`, `Label`, `Separator`, and `Collapsible` + `ItemGroup` (`Item`, `ItemHeader`, `ItemMedia`, `ItemContent`, `ItemTitle`, `ItemDescription`, `ItemActions`) for cited email evidence cards.

## Retrieval, Storage & Context Engineering

BM25 retrieval stays in the bundled SQLite index and is always available. When the user enables remote embedding indexing, bounded email documents are sent in batches to the configured OpenAI-compatible embeddings endpoint and vectors are stored separately by message and model. Questions then use reciprocal-rank fusion over BM25 and cosine-similarity results; disabling the switch uses BM25 alone. The explicitly local “Search only” action never generates a remote query embedding.

Indexed sources are tracked by their archive path. Archive and folder enablement are non-destructive query filters applied consistently to FTS and vector candidates. Removing a source deletes only its indexed messages; SQLite cascades the deletion to their vectors and the FTS delete trigger removes their terms. The source PST remains read-only and untouched.

For each question, the selected results are converted to rich, bounded, numbered evidence documents and attached as static context to a Rig agent. Context engineering ensures:
- **Fair budget allocation**: Dynamic per-document budgeting distributes the character limit across all retrieved hits, preventing a single long email from starving other relevant evidence.
- **Email cleaning**: Strips legal boilerplate disclaimers, condenses deep nested quote chains (`> > >`), and collapses whitespace bloat to maximize evidence density.
- **Rich metadata injection**: Evidence cards and prompt context include attachment names, conversation thread identifiers, Laya/LLM Judge labels with confidence scores, source PST archives, and message IDs.
- **Thread continuity expansion**: Retrieves surrounding thread companion messages for high-confidence conversation threads so the agent observes both questions and answers.
- **Multi-turn conversational memory & query contextualization**: Preserves recent dialogue turns and contextualizes anaphoric follow-up questions before retrieval.
- **Structured agent preamble**: Instructs the agent on temporal reasoning (chronological progression and superseded decisions), explicit `[Email N]` bracketed citations, metadata awareness, and strict grounding limits.

Rig targets the configured OpenAI-compatible chat-completions endpoint; the application still validates HTTPS (with the existing exact `127.0.0.1` development exception) before constructing either model client.

The `MailSource` boundary prevents parser-specific identifiers and lifetimes from entering storage, retrieval, or UI code. Index writes are transactional and `INSERT OR IGNORE` on a SHA-256 content hash makes repeated imports idempotent. Production incremental indexing should additionally persist the PST file identity, size, modification time, and stable node identifiers; a changed file must resume only where the chosen parser proves those identifiers stable.

## Gates before production

1. Convert the working recursive PST traversal to a streaming callback; the current adapter returns all messages in a `Vec`. Batch 250–1000 normalized messages per SQLite transaction for multi-GB archives.
2. Test Unicode and ANSI PSTs, damaged archives, password-protected archives, large files, nested folders, RTF/HTML bodies, duplicated messages, and Outlook-held files against a representative corpus.
3. Sign the executable and generate an SBOM/license inventory. Confirm every dependency can be statically distributed.
4. Add Windows Credential Manager storage for API secrets and a settings screen for private-root CA certificates. Never disable certificate validation.
5. Add explicit index deletion, cancellation, progress, disk-space checks, and crash-safe checkpointing.
6. Measure working set and index throughput on the lowest supported office PC.
7. Build and smoke-test the exact single executable on a clean Windows 11 VM with no developer runtime or Internet access.

## Locked PST files

Open read-only with permissive Windows share flags first. If a consistent read cannot be obtained, present an explicit local-snapshot action. VSS availability and authorization vary by organization and must be proven in the target environment; never modify the source PST and never pretend a normal byte copy is transactionally consistent while Outlook is writing.
