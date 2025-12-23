# Claude Code Zed - Native CLI Integration

> **This is an actively maintained fork** of [jiahaoxiang2000/claude-code-zed](https://github.com/jiahaoxiang2000/claude-code-zed).
>
> **Why this fork exists:** Zed's built-in ACP (Agent Client Protocol) integration has limitations - no hooks support, subset of slash commands, no past message editing. This project provides **full Claude Code CLI features** with Zed editor context awareness.

## Credits

**Original Author:** [jiahaoxiang2000](https://github.com/jiahaoxiang2000) - Thank you for creating the foundation of this integration!

The original repository was archived in August 2025. This fork continues development to provide a native CLI-to-Zed bridge without ACP limitations.

---

## What This Does

Bridges **Claude Code CLI** (full features) with **Zed Editor** (context awareness):

```
Zed Editor ←→ WebSocket Server ←→ Claude Code CLI
     │                                    │
     └── Selection, file context          └── Full CLI: hooks, slash commands, etc.
```

**You get:**
- Full Claude Code CLI with all features (hooks, custom commands, etc.)
- Zed knows what you've selected and which file you're in
- Claude can see your selection context when you ask questions
- Auto-starts when Zed opens supported files

## Quick Start

### Prerequisites
- Zed editor (0.202.5+)
- Claude Code CLI
- Rust toolchain

### Installation

```bash
# 1. Clone this repo
git clone https://github.com/etanhey/claude-code-zed.git
cd claude-code-zed

# 2. Install WASM target (one-time)
rustup target add wasm32-wasip1

# 3. Build and deploy server
make dev-build

# 4. Install Zed extension
#    In Zed: Cmd+Shift+P → "zed: install dev extension"
#    Navigate to: claude-code-zed/claude-code-extension
```

### Usage

1. **Open any supported file in Zed** (`.ts`, `.js`, `.py`, `.rs`, `.md`, etc.)
2. **The server auto-starts** (creates `~/.claude/ide/59792.lock`)
3. **Run Claude Code CLI:** `claude`
4. **Connect:** Type `/ide` and select `claude-code-server`
5. **Test:** Select text in Zed, ask Claude "what did I select?"

## Current Features

| Feature | Status |
|---------|--------|
| Text selection sharing | ✅ Working |
| File path context | ✅ Working |
| Line number tracking | ✅ Working |
| Auto-start with Zed | ✅ Working |
| UTF-16 emoji handling | ✅ Fixed |
| LSP diagnostics | ❌ Not implemented |
| Claude → Zed editing | ❌ Not implemented |

## Supported Languages

TypeScript, JavaScript, TSX, Python, Rust, Ruby, Markdown, LaTeX, Typst, Elixir, Erlang, Kotlin

Add more in `claude-code-extension/extension.toml`.

## Development

```bash
make dev-build    # Build + deploy to Zed
make dev-test     # Build + testing instructions
make dev-clean    # Remove deployment
make status       # Check deployment status
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for detailed development guide.

See [IMPROVEMENTS.md](IMPROVEMENTS.md) for planned features and contribution ideas.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       ZED EDITOR                            │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  claude-code-extension (WASM)                         │  │
│  │  - Downloads/starts server binary                     │  │
│  │  - Tracks selections via LSP                          │  │
│  └─────────────────────────┬─────────────────────────────┘  │
└────────────────────────────┼────────────────────────────────┘
                             │ LSP (stdin/stdout)
                             ▼
┌─────────────────────────────────────────────────────────────┐
│  claude-code-server (Native Rust)                           │
│  - LSP server (receives selection events)                   │
│  - WebSocket server (exposes to Claude CLI)                 │
│  - Lock file management (~/.claude/ide/[port].lock)         │
└────────────────────────────┬────────────────────────────────┘
                             │ WebSocket + Lock file discovery
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    CLAUDE CODE CLI                          │
│  - Discovers server via lock file                           │
│  - Full CLI features (hooks, commands, etc.)                │
│  - Receives selection context from Zed                      │
└─────────────────────────────────────────────────────────────┘
```

## Troubleshooting

**Extension won't install:**
```bash
rustup target add wasm32-wasip1
rustup update
```

**Server not starting:**
- Check Zed logs: `zed --foreground .`
- Verify lock file: `ls ~/.claude/ide/`
- Restart Zed after `make dev-build`

**Claude can't see selection:**
- Ensure file type is supported
- Try `/ide` again in Claude CLI
- Check WebSocket connection in server logs

## License

MIT - See [LICENSE](LICENSE)

---

**Original project:** [jiahaoxiang2000/claude-code-zed](https://github.com/jiahaoxiang2000/claude-code-zed)
