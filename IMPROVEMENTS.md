# Improvements & Feature Roadmap

This document outlines potential improvements, new features, and enhancement ideas for the claude-code-zed integration. Contributions welcome!

---

## Priority Legend

- **P0** - Critical / High Impact
- **P1** - Important / Medium Impact
- **P2** - Nice to Have / Low Impact
- **Effort**: S (small, <1 day), M (medium, 1-3 days), L (large, 1+ week)

---

## 1. Bidirectional Communication (Claude → Zed)

Currently the integration is primarily one-way (Zed → Claude). Enabling Claude to send commands back to Zed would unlock powerful features.

### 1.1 Open File in Zed [P0, Effort: M]

**What:** When Claude references a file, clicking it opens in Zed at the correct line.

**Implementation:**
- Implement `openFile` tool handler in `mcp.rs` (currently stubbed)
- Use Zed's LSP `workspace/applyEdit` or custom command
- Pass file path, line number, and optional selection range

**Files to modify:**
- `claude-code-server/src/mcp.rs` - Implement actual file opening
- `claude-code-extension/src/lib.rs` - Handle incoming commands

### 1.2 Apply Diffs in Zed [P0, Effort: L]

**What:** When Claude suggests code changes, apply them directly in Zed with a diff view.

**Implementation:**
- Implement `openDiff` tool handler properly
- Create temporary file with new content
- Trigger Zed's diff view between original and modified
- Handle accept/reject callbacks

**Protocol reference:** See `openDiff` in [claudecode.nvim PROTOCOL.md](https://github.com/coder/claudecode.nvim/blob/main/PROTOCOL.md)

### 1.3 Insert Text at Cursor [P1, Effort: M]

**What:** Claude can insert code snippets at the current cursor position.

**Implementation:**
- Track cursor position (not just selection)
- Implement `insertText` command
- Use LSP `workspace/applyEdit` with insert operation

---

## 2. LSP Diagnostics Integration

### 2.1 Send Diagnostics to Claude [P0, Effort: L]

**What:** Share TypeScript errors, ESLint warnings, etc. with Claude for context-aware debugging.

**Current state:** LSP client in extension doesn't expose diagnostics.

**Implementation options:**

**Option A: Zed Extension API (if available)**
- Check if Zed's extension API exposes `textDocument/publishDiagnostics`
- Subscribe to diagnostic events in `lib.rs`
- Forward to server via custom LSP notification

**Option B: External LSP Client**
- Run a separate LSP client for the target language
- Parse diagnostics and merge with selection context
- More complex but doesn't depend on Zed API

**Files to modify:**
- `claude-code-extension/src/lib.rs` - Diagnostic subscription
- `claude-code-server/src/lsp.rs` - New notification type
- `claude-code-server/src/mcp.rs` - `getDiagnostics` tool

### 2.2 Diagnostic Filtering [P2, Effort: S]

**What:** Filter diagnostics by severity (errors only, warnings, hints).

**Implementation:**
- Add configuration option
- Filter in server before sending to Claude

---

## 3. Multi-File Context

### 3.1 Open Editors List [P1, Effort: M]

**What:** Send list of all open files in Zed to Claude for broader context.

**Implementation:**
- Implement `getOpenEditors` tool (currently returns empty)
- Track document open/close events in LSP server
- Maintain in-memory list of open documents

### 3.2 Workspace Symbol Search [P1, Effort: M]

**What:** Allow Claude to search for symbols across the workspace.

**Implementation:**
- Forward `workspace/symbol` requests to Zed's built-in LSP
- Parse and return results to Claude
- Enable "find all usages of X" type queries

### 3.3 Project Structure Context [P2, Effort: S]

**What:** Send project file tree to Claude for navigation context.

**Implementation:**
- Walk directory tree (respecting .gitignore)
- Send as context with selection
- Cache and invalidate on file system changes

---

## 4. Enhanced Selection Tracking

### 4.1 Multiple Selections [P1, Effort: M]

**What:** Support Zed's multiple cursor selections.

**Current state:** Only tracks single selection.

**Implementation:**
- Modify `SelectionChangedNotification` to include array of selections
- Update Claude context to show all selected regions

### 4.2 Selection History [P2, Effort: S]

**What:** Remember recent selections for "what did I select before?" queries.

**Implementation:**
- Ring buffer of last N selections (configurable)
- Timestamp each selection
- Expose via `getSelectionHistory` tool

### 4.3 Smart Selection Expansion [P2, Effort: M]

**What:** Automatically expand selection to semantic boundaries (function, class, block).

**Implementation:**
- Use LSP `textDocument/selectionRange` capability
- Offer "expand to function" style commands
- Requires language-aware parsing

---

## 5. Performance & Reliability

### 5.1 Connection Resilience [P0, Effort: M]

**What:** Auto-reconnect when connection drops, don't require `/ide` again.

**Implementation:**
- Implement heartbeat/ping in WebSocket
- Auto-reconnect with exponential backoff
- Persist auth token for session

### 5.2 Lazy Loading [P1, Effort: S]

**What:** Don't read entire file on every selection, use ranges.

**Current state:** `read_text_from_range` reads full file each time.

**Implementation:**
- Cache file contents with modification timestamp
- Invalidate on `didChange` events
- Only read changed portions

### 5.3 Debounced Selection Events [P1, Effort: S]

**What:** Don't spam selection events on rapid cursor movement.

**Implementation:**
- Add configurable debounce (default 100ms)
- Only send after selection stabilizes
- Reduces WebSocket traffic significantly

---

## 6. Developer Experience

### 6.1 Status Bar Integration [P1, Effort: M]

**What:** Show connection status in Zed's status bar.

**Implementation:**
- Use Zed's status bar API (if available)
- Show: Connected/Disconnected, last selection sent
- Click to reconnect or show logs

### 6.2 Command Palette Actions [P1, Effort: M]

**What:** Add Zed commands like "Send selection to Claude", "Connect to Claude".

**Implementation:**
- Define commands in extension manifest
- Implement command handlers
- Keyboard shortcuts (configurable)

### 6.3 Configuration UI [P2, Effort: M]

**What:** Settings panel for port, auto-connect, debug logging, etc.

**Implementation:**
- Use Zed's settings API
- Persist in `~/.config/zed/settings.json`
- Live reload on config change

---

## 7. Language-Specific Enhancements

### 7.1 TypeScript/JavaScript [P1, Effort: M]

- Send import statements with selection
- Include type definitions for selected symbols
- Forward TSServer quick fixes to Claude

### 7.2 Rust [P2, Effort: M]

- Include trait implementations
- Send cargo check output
- Macro expansion context

### 7.3 Python [P2, Effort: M]

- Virtual environment detection
- Type stub information
- Docstring context

---

## 8. Security & Privacy

### 8.1 Selection Filtering [P1, Effort: S]

**What:** Don't send sensitive content (env files, secrets).

**Implementation:**
- Configurable file/pattern blacklist
- Warn when sending from sensitive files
- Option to require confirmation

### 8.2 Token Rotation [P2, Effort: S]

**What:** Rotate auth token periodically.

**Implementation:**
- Generate new token on schedule
- Update lock file atomically
- Handle reconnection with new token

---

## 9. Testing & Quality

### 9.1 Integration Tests [P1, Effort: M]

**What:** Automated tests for the full flow.

**Implementation:**
- Mock Zed extension environment
- Test WebSocket protocol
- CI pipeline with GitHub Actions

### 9.2 Protocol Conformance Tests [P1, Effort: S]

**What:** Ensure MCP implementation matches spec.

**Implementation:**
- Test each tool against expected behavior
- Validate JSON-RPC responses
- Compare with reference implementations

---

## 10. Distribution & Installation

### 10.1 Zed Extension Marketplace [P0, Effort: M]

**What:** Publish to official Zed extensions.

**Requirements:**
- Clean up extension metadata
- Binary hosting (GitHub releases works)
- Documentation in extension format

### 10.2 Homebrew Formula [P2, Effort: S]

**What:** `brew install claude-code-zed`

**Implementation:**
- Create formula for server binary
- Auto-install extension or provide instructions

### 10.3 One-Line Install Script [P1, Effort: S]

**What:** `curl ... | sh` style installer.

**Implementation:**
- Detect platform
- Download binary
- Install extension
- Configure automatically

---

## Implementation Priority Matrix

| Feature | Impact | Effort | Priority |
|---------|--------|--------|----------|
| Apply Diffs in Zed | High | Large | P0 |
| Open File in Zed | High | Medium | P0 |
| LSP Diagnostics | High | Large | P0 |
| Connection Resilience | High | Medium | P0 |
| Zed Extension Marketplace | High | Medium | P0 |
| Open Editors List | Medium | Medium | P1 |
| Debounced Selection | Medium | Small | P1 |
| Status Bar Integration | Medium | Medium | P1 |
| One-Line Installer | Medium | Small | P1 |
| Multiple Selections | Medium | Medium | P1 |
| Selection History | Low | Small | P2 |
| Configuration UI | Low | Medium | P2 |

---

## Contributing

1. Pick an item from this list
2. Open an issue to discuss approach
3. Fork, implement, test
4. Submit PR with before/after demo

For large features (Effort: L), consider opening a discussion first to align on design.

---

## Research Needed

These items need more investigation before implementation:

- **Zed Extension API capabilities** - What's exposed for diagnostics, status bar, commands?
- **Zed's LSP client behavior** - Can we intercept or extend it?
- **MCP protocol evolution** - Are there new capabilities we should support?
- **ACP vs native comparison** - Detailed feature matrix to ensure we cover all use cases

---

## Ideas from the Community

Have an idea not listed here? Open an issue with:
- Use case description
- Expected behavior
- Any technical considerations

We'll add promising ideas to this roadmap!
