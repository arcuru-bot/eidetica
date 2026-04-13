# Chat Example — Feature Backlog

Tracked improvements for the eidetica chat TUI, roughly in priority order.

## Completed

- [x] Fix q/Esc quit — use Ctrl+C/Ctrl+Q instead
- [x] Full input line editor (cursor, Home/End, Ctrl+A/E/K/U/W, word nav)
- [x] Input history (Up/Down)
- [x] Nick list sidebar with consistent hash-based colors
- [x] Slash commands (/nick, /me, /clear, /topic, /users, /help, /quit)
- [x] Help overlay
- [x] Smart auto-scroll (pin-to-bottom, unread indicator)
- [x] PageUp/PageDown, Ctrl+Up/Down scroll
- [x] Action messages (/me) and system messages
- [x] CLI subcommands (create, send, messages, tui) for scripting
- [x] Unit tests for InputLine and ChatMessage

## High Priority

- [x] ~~**Tab-completion for nicks**~~ — Tab cycles through matching nicks, IRC-style `: ` suffix
- [x] ~~**Message wrapping**~~ — Paragraph with Wrap, scroll math accounts for wrapped lines
- [x] ~~**Persistent storage**~~ — Sqlite backend with `--data-dir` flag
- [x] ~~**Notifications**~~ — terminal bell + title update on new synced messages
- [x] ~~**Nick mentions**~~ — bold yellow highlight with word boundary matching
- [x] ~~**URL detection**~~ — underlined blue for http/https URLs
- [x] ~~**Timestamps toggle**~~ — `/timestamps` or `/ts` to show/hide

## Medium Priority

- [ ] **Multi-line input** — Shift+Enter or paste support

## Bigger Lifts

- [ ] **Multiple rooms** — `/join`, `/part`, room tabs or switchable room list
- [ ] **Connection status** — show sync state (connected/syncing/disconnected) in topic bar
- [ ] **Message search** — `/search <query>` to filter/highlight messages
- [x] ~~**Room encryption**~~ — AES-256-GCM via PasswordStore, /encrypt and /decrypt commands, --password CLI flag
