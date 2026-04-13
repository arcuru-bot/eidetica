# Eidetica Chat

A mostly vibe-coded and human reviewed Chat application demonstrating usage of the API and sync capabilities. It's to show how to use the API and as a test-bed for the ergonomics of Eidetica's API surface.

It's a Terminal User Interface (TUI) chat application with room-based messaging, user accounts, and peer-to-peer synchronization.

## Overview

This example showcases:

- **User accounts**: Automatic passwordless user creation and management
- **Room-based chat**: Create and join multiple chat rooms (each room is a separate database)
- **Multi-transport sync**: Choose between HTTP (simple client-server) or Iroh (P2P with NAT traversal)
- **Connection sharing**: Share room addresses to invite others
- **Automatic sync**: Messages sync in real-time between connected peers
- **IRC-style TUI**: Full-featured terminal interface with nick list, slash commands, input history, and help overlay

## Quick Start

### Create a New Room

```bash
# From the repo root, run:
cd examples/chat

# Create a new room (default Iroh transport)
cargo run -- --username alice

# Or use HTTP transport
cargo run -- --username alice --transport http
```

When you create a new room, the app will:

1. Display the room address that others can use to join
2. Wait for you to press Enter before starting the chat

Example output:

```
Eidetica Chat Room Created!

Room Address: eidetica:?db=bafyrei...&pr=iroh:endpoint...
Username: alice

Share this address with others to invite them.
Press Enter to start chatting...
```

### Join an Existing Room

```bash
# Connect to a room using its address
cargo run -- <room_address> --username bob
```

The app will connect to the specified room and start the chat interface immediately.

### Chat Interface

Once in a room, you'll see:

- **Topic bar**: Room name and shareable address at the top
- **Messages**: Chat history in the center, with timestamps and colored nicks
- **Nick list**: Known users on the right sidebar
- **Input field**: Type messages at the bottom

#### Keyboard Shortcuts

| Key | Action |
|---|---|
| `Ctrl+C` / `Ctrl+Q` | Quit |
| `Enter` | Send message |
| `Up` / `Down` | Browse input history |
| `Ctrl+Up` / `Ctrl+Down` | Scroll messages (one line) |
| `PageUp` / `PageDown` | Scroll messages (fast) |
| `Left` / `Right` | Move cursor |
| `Ctrl+Left` / `Ctrl+Right` | Move cursor by word |
| `Home` / `Ctrl+A` | Start of line |
| `End` / `Ctrl+E` | End of line |
| `Ctrl+K` | Kill to end of line |
| `Ctrl+U` | Kill to start of line |
| `Ctrl+W` | Kill word back |

#### Slash Commands

| Command | Description |
|---|---|
| `/nick <name>` | Change your nickname |
| `/me <action>` | Send an action message (e.g., `/me waves`) |
| `/clear` | Clear the message display (messages persist in DB) |
| `/topic` | Show the current room topic |
| `/users` / `/names` | List known users |
| `/timestamps` / `/ts` | Toggle timestamp display |
| `/help` / `/?` | Toggle help overlay |
| `/quit` / `/q` | Quit |

## CLI Subcommands

The chat app supports both interactive (TUI) and non-interactive (CLI) usage:

```bash
# Create a room, print the ticket to stdout
cargo run -- create --username alice
# => eidetica:?db=bafyrei...&pr=iroh:endpoint...

# Send a message to a room
cargo run -- send <TICKET> "hello from the CLI" --username alice

# List messages (last 50 by default)
cargo run -- messages <TICKET>

# Tail messages continuously
cargo run -- messages <TICKET> --follow

# Output messages as JSON
cargo run -- messages <TICKET> --json

# Open the interactive TUI (default when no subcommand)
cargo run -- tui <TICKET> --username alice
cargo run -- --username alice   # no subcommand = TUI + create new room
```

### Global Options

```
  -u, --username <USERNAME>      Username for the chat session (default: $USER or "Anonymous")
  -v, --verbose                  Enable verbose debug output
      --transport <TRANSPORT>    Transport to use: 'http' or 'iroh' (default: iroh)
      --data-dir <PATH>          Directory for SQLite database
                                 (default: ~/.local/share/eidetica-chat)
```

### Persistent Storage

Data is stored in a SQLite database at `<data-dir>/chat.db`. Multiple CLI invocations
sharing the same `--data-dir` share the same rooms and messages with no sync required.

## Connecting with Others

1. **Create a room** - Run without a room address to create a new room
2. **Copy the room address** - The address is displayed after creation
3. **Share the address** - Send it to others via any communication channel
4. **Others join** - They run the app with your room address as an argument

## Example Workflow

### Two-User Chat Session

**Terminal 1 (Alice - Room Creator):**

```bash
cd examples/chat
cargo run -- --username alice

# App displays:
# Eidetica Chat Room Created!
# Room Address: eidetica:?db=bafyrei...&pr=iroh:endpoint...
# Username: alice
#
# Share this address with others to invite them.
# Press Enter to start chatting...

# Copy the ticket URL, share it with Bob, then press Enter
```

**Terminal 2 (Bob - Room Joiner):**

```bash
cd examples/chat

# Paste Alice's ticket URL as the first argument
cargo run -- 'eidetica:?db=bafyrei...&pr=iroh:endpoint...' --username bob

# Chat interface starts immediately - start chatting!
```

### Using Different Transports

**HTTP (for local testing):**

```bash
cargo run -- --username alice --transport http
```

**Iroh (for P2P across networks):**

```bash
cargo run -- --username alice --transport iroh
```

## Architecture

The example demonstrates Eidetica's key components:

- **Instance**: Main database system managing storage and sync
- **User**: Passwordless user account with automatic key management
- **Database**: Each room is a separate database with its own authentication
- **Table Store**: Messages stored in a `Table<ChatMessage>` store within each room
- **Sync System**: Automatic peer synchronization with configurable transports
- **Bootstrap Protocol**: Automatic database access requests when joining rooms
