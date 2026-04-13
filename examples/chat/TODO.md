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

## High Priority

- [ ] **Tab-completion for nicks** — `@bo<Tab>` → `@bob`, cycle through matches
- [ ] **Message wrapping** — long messages overflow; wrap and fix scroll math for wrapped lines
- [ ] **Persistent storage** — switch from InMemory to Sqlite backend so history survives restarts
- [ ] **Notifications** — terminal bell or title update on new messages when unfocused

## Medium Priority

- [ ] **Nick mentions** — highlight messages containing your username
- [ ] **URL detection** — highlight URLs in messages with distinct color
- [ ] **Timestamps toggle** — `/timestamps` to hide/show for narrow terminals
- [ ] **Multi-line input** — Shift+Enter or paste support

## Bigger Lifts

- [ ] **Multiple rooms** — `/join`, `/part`, room tabs or switchable room list
- [ ] **Connection status** — show sync state (connected/syncing/disconnected) in topic bar
- [ ] **Message search** — `/search <query>` to filter/highlight messages
- [ ] **Room encryption** — leverage eidetica's PasswordStore for encrypted rooms
