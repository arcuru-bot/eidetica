use crate::models::ChatMessage;
use eidetica::{
    Database, Instance, Result,
    auth::{AuthKey, Permission, types::Permission as PermissionType},
    crdt::Doc,
    store::Table,
    sync::{
        DatabaseTicket, SyncError,
        transports::{http::HttpTransport, iroh::IrohTransport},
    },
    user::{User, types::SyncSettings},
};
use ratatui::widgets::ScrollbarState;
use std::collections::BTreeSet;
use tracing::{debug, info};

/// Input line editor with cursor support
pub struct InputLine {
    pub text: String,
    pub cursor: usize,
}

impl InputLine {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            // Find the previous char boundary
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
            self.text.drain(self.cursor..next);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.text[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.text.len());
        }
    }

    pub fn move_word_left(&mut self) {
        // Skip whitespace, then skip word chars
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        if trimmed.is_empty() {
            self.cursor = 0;
            return;
        }
        // Find last space in trimmed portion
        self.cursor = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
    }

    pub fn move_word_right(&mut self) {
        let after = &self.text[self.cursor..];
        // Skip current word chars, then skip whitespace
        let skip_word = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let rest = &after[skip_word..];
        let skip_space = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        self.cursor += skip_word + skip_space;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn kill_to_end(&mut self) {
        self.text.truncate(self.cursor);
    }

    pub fn kill_to_start(&mut self) {
        self.text.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn kill_word_back(&mut self) {
        let old_cursor = self.cursor;
        self.move_word_left();
        self.text.drain(self.cursor..old_cursor);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Cursor position in characters (for display)
    pub fn cursor_chars(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

pub struct App {
    pub user: User,
    pub instance: Instance,

    // Current room state
    pub current_room: Option<Database>,
    pub current_room_address: Option<String>,
    pub current_room_name: Option<String>,

    // Chat state
    pub input: InputLine,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>, // None = not browsing history
    pub history_stash: String,        // saves current input when browsing history
    pub messages: Vec<ChatMessage>,
    pub scroll_state: ScrollbarState,
    pub scroll_position: usize,
    pub pinned_to_bottom: bool, // auto-scroll only when pinned

    // User info
    pub username: String,

    // Known users (extracted from messages)
    pub known_users: BTreeSet<String>,

    // Sync state
    pub server_running: bool,
    pub transport: String,

    pub status_message: Option<String>,
    pub should_quit: bool,
    pub show_help: bool,
    pub show_timestamps: bool,

    // Tab completion state
    pub tab_completion: Option<TabCompletion>,

    // Notification state
    pub needs_bell: bool,
}

/// Tracks an in-progress tab completion cycle
pub struct TabCompletion {
    /// The partial text being completed
    pub prefix: String,
    /// Position in input where the prefix starts
    pub start: usize,
    /// Matching candidates
    pub candidates: Vec<String>,
    /// Current index into candidates
    pub index: usize,
}

impl App {
    pub fn new(instance: Instance, user: User, username: String, transport: &str) -> Result<Self> {
        let mut known_users = BTreeSet::new();
        known_users.insert(username.clone());

        Ok(Self {
            user,
            instance,
            current_room: None,
            current_room_address: None,
            current_room_name: None,
            input: InputLine::new(),
            input_history: Vec::new(),
            history_index: None,
            history_stash: String::new(),
            messages: Vec::new(),
            scroll_state: ScrollbarState::default(),
            scroll_position: 0,
            pinned_to_bottom: true,
            username,
            known_users,
            server_running: false,
            transport: transport.to_string(),
            status_message: None,
            should_quit: false,
            show_help: false,
            show_timestamps: true,
            tab_completion: None,
            needs_bell: false,
        })
    }

    pub async fn create_room(&mut self, name: &str) -> Result<()> {
        let mut settings = Doc::new();
        settings.set("name", name);

        let key_id = self.user.get_default_key()?;
        let database = self.user.create_database(settings, &key_id).await?;

        // Add global write permission
        let tx = database.new_transaction().await?;
        let settings_store = tx.get_settings()?;
        let global_key = AuthKey::active(None, Permission::Write(0));
        settings_store.set_global_auth_key(global_key).await?;
        tx.commit().await?;

        // Enable sync
        let database_id = database.root_id().clone();
        self.user
            .track_database(
                database_id,
                &key_id,
                SyncSettings::on_commit().with_interval(2),
            )
            .await?;

        self.enter_room(database).await?;

        if let Some(addr) = &self.current_room_address {
            self.status_message = Some(format!("Room created! Share this address: {addr}"));
        }

        Ok(())
    }

    pub async fn enter_room(&mut self, database: Database) -> Result<()> {
        if !self.server_running {
            self.start_server().await?;
        }

        let room_address = if let Some(sync) = self.instance.sync() {
            match sync.create_ticket(database.root_id()).await {
                Ok(ticket) => ticket.to_string(),
                Err(e) => {
                    tracing::warn!("Failed to create ticket with addresses: {e}");
                    DatabaseTicket::new(database.root_id().clone()).to_string()
                }
            }
        } else {
            DatabaseTicket::new(database.root_id().clone()).to_string()
        };

        let room_name = database.get_name().await.ok();

        self.current_room = Some(database);
        self.current_room_address = Some(room_address);
        self.current_room_name = room_name;
        self.load_messages().await?;

        Ok(())
    }

    async fn start_server(&mut self) -> Result<()> {
        if let Some(sync) = self.instance.sync() {
            match self.transport.as_str() {
                "http" => {
                    sync.register_transport("http", HttpTransport::builder().bind("127.0.0.1:0"))
                        .await?;
                    sync.accept_connections().await?;
                }
                "iroh" => {
                    sync.register_transport("iroh", IrohTransport::builder())
                        .await?;
                    sync.accept_connections().await?;
                }
                _ => {
                    return Err(SyncError::Network(format!(
                        "Unknown transport: {}",
                        self.transport
                    ))
                    .into());
                }
            }
            self.server_running = true;
        }
        Ok(())
    }

    /// Open a room from a ticket, preferring local DB if available (no sync).
    /// Returns true if opened locally, false if it needed remote sync.
    pub async fn open_room(&mut self, room_address: &str) -> Result<bool> {
        let ticket: DatabaseTicket = room_address
            .parse()
            .map_err(|e| SyncError::Network(format!("Invalid ticket URL: {e}")))?;
        let room_id = ticket.database_id().clone();

        // Check if the database exists locally in the backend
        let exists_locally = match self.user.backend().get(&room_id).await {
            Ok(_) => true,
            Err(e) if e.is_not_found() => false,
            Err(e) => return Err(e),
        };

        if exists_locally {
            // Ensure this user has a key tracking the database
            let key_id = self.user.get_default_key()?;
            // track_database is idempotent — safe to call if already tracked
            let _ = self
                .user
                .track_database(
                    room_id.clone(),
                    &key_id,
                    SyncSettings::on_commit().with_interval(2),
                )
                .await;

            match self.user.open_database(&room_id).await {
                Ok(database) => {
                    self.current_room = Some(database);
                    self.current_room_address = Some(room_address.to_string());
                    self.load_messages().await?;
                    return Ok(true);
                }
                Err(e) => {
                    debug!("Local open failed, falling back to remote: {e}");
                }
            }
        }

        // Not local or local open failed — do full remote connect
        self.connect_to_room(room_address).await?;
        Ok(false)
    }

    pub async fn connect_to_room(&mut self, room_address: &str) -> Result<()> {
        if !self.server_running {
            self.start_server().await?;
        }

        let ticket: DatabaseTicket = room_address
            .parse()
            .map_err(|e| SyncError::Network(format!("Invalid ticket URL: {e}")))?;
        let room_id = ticket.database_id().clone();
        debug!(room_id = %room_id, addresses = ?ticket.addresses(), "Parsed ticket");

        let is_bootstrap = match self.user.backend().get(&room_id).await {
            Ok(_) => false,
            Err(e) if e.is_not_found() => true,
            Err(e) => return Err(e),
        };

        let sync = self
            .instance
            .sync()
            .ok_or_else(|| SyncError::Network("No sync instance available".into()))?;

        if is_bootstrap {
            let key_id = self.user.get_default_key()?;
            info!("Starting bootstrap sync for room {room_id}");

            self.user
                .request_database_access(&sync, &ticket, &key_id, PermissionType::Write(5))
                .await?;

            self.user
                .track_database(
                    room_id.clone(),
                    &key_id,
                    SyncSettings::on_commit().with_interval(2),
                )
                .await?;
        } else {
            sync.sync_with_ticket(&ticket).await?;
        }

        let mut attempts = 0;
        let database = loop {
            attempts += 1;
            match self.user.open_database(&room_id).await {
                Ok(db) => break db,
                Err(e) if attempts >= 30 => {
                    return Err(SyncError::Network(format!(
                        "Timed out waiting for room to sync: {e}"
                    ))
                    .into());
                }
                Err(_) => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        };

        self.current_room = Some(database);
        self.current_room_address = Some(room_address.to_string());
        self.load_messages().await?;

        Ok(())
    }

    pub async fn load_messages(&mut self) -> Result<()> {
        if let Some(database) = &self.current_room {
            let txn = database.new_transaction().await?;
            let store = txn.get_store::<Table<ChatMessage>>("messages").await?;

            let entries = store.search(|_| true).await?;
            let mut messages: Vec<ChatMessage> = entries.into_iter().map(|(_, msg)| msg).collect();
            messages.sort_by_key(|a| a.timestamp);

            // Update known users
            for msg in &messages {
                if msg.author != "*" {
                    self.known_users.insert(msg.author.clone());
                }
            }

            self.messages = messages;
            if self.pinned_to_bottom {
                self.scroll_to_bottom();
            }
        }
        Ok(())
    }

    pub async fn send_message(&mut self) -> Result<()> {
        if self.input.is_empty() || self.current_room.is_none() {
            return Ok(());
        }

        let text = self.input.text.trim().to_string();

        // Save to input history
        if !text.is_empty() {
            // Don't duplicate the last history entry
            if self.input_history.last().map(|s| s.as_str()) != Some(&text) {
                self.input_history.push(text.clone());
            }
        }
        self.history_index = None;

        // Handle slash commands
        if text.starts_with('/') {
            return self.handle_command(&text).await;
        }

        let message = ChatMessage::new(self.username.clone(), text);

        if let Some(database) = &self.current_room {
            let txn = database.new_transaction().await?;
            let store = txn.get_store::<Table<ChatMessage>>("messages").await?;
            store.insert(message.clone()).await?;
            txn.commit().await?;

            self.messages.push(message);
            self.input.clear();
            self.pinned_to_bottom = true;
            self.scroll_to_bottom();
        }

        Ok(())
    }

    async fn handle_command(&mut self, text: &str) -> Result<()> {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd.as_str() {
            "/quit" | "/q" => {
                self.should_quit = true;
            }
            "/nick" => {
                if arg.is_empty() {
                    self.status_message = Some(format!("Current nick: {}", self.username));
                } else {
                    let old = self.username.clone();
                    self.username = arg.to_string();
                    self.known_users.remove(&old);
                    self.known_users.insert(self.username.clone());
                    self.status_message = Some(format!("Nick changed: {old} -> {}", self.username));

                    // Send a system message about the nick change
                    let msg = ChatMessage::new(
                        "*".to_string(),
                        format!("{old} is now known as {}", self.username),
                    );
                    if let Some(database) = &self.current_room {
                        let txn = database.new_transaction().await?;
                        let store = txn.get_store::<Table<ChatMessage>>("messages").await?;
                        store.insert(msg.clone()).await?;
                        txn.commit().await?;
                        self.messages.push(msg);
                    }
                }
            }
            "/me" => {
                if !arg.is_empty() {
                    let msg = ChatMessage::new("*".to_string(), format!("{} {arg}", self.username));
                    if let Some(database) = &self.current_room {
                        let txn = database.new_transaction().await?;
                        let store = txn.get_store::<Table<ChatMessage>>("messages").await?;
                        store.insert(msg.clone()).await?;
                        txn.commit().await?;
                        self.messages.push(msg);
                        self.pinned_to_bottom = true;
                        self.scroll_to_bottom();
                    }
                }
            }
            "/clear" => {
                // Clear local display only (messages persist in DB)
                self.messages.clear();
                self.scroll_position = 0;
                self.status_message = Some("Display cleared (messages persist in database)".into());
            }
            "/topic" => {
                if arg.is_empty() {
                    let name = self
                        .current_room_name
                        .as_deref()
                        .unwrap_or("(no topic set)");
                    self.status_message = Some(format!("Topic: {name}"));
                } else {
                    self.status_message = Some("Changing topic not yet supported".into());
                }
            }
            "/timestamps" | "/ts" => {
                self.show_timestamps = !self.show_timestamps;
                self.status_message = Some(format!(
                    "Timestamps {}",
                    if self.show_timestamps { "on" } else { "off" }
                ));
            }
            "/help" | "/?" => {
                self.show_help = !self.show_help;
            }
            "/users" | "/names" => {
                let names: Vec<&str> = self.known_users.iter().map(|s| s.as_str()).collect();
                self.status_message = Some(format!("Users: {}", names.join(", ")));
            }
            _ => {
                self.status_message = Some(format!("Unknown command: {cmd}. Try /help"));
            }
        }

        self.input.clear();
        Ok(())
    }

    /// Start or cycle tab completion at the current cursor position
    pub fn tab_complete(&mut self) {
        if let Some(ref mut tc) = self.tab_completion {
            // Cycle to next candidate
            tc.index = (tc.index + 1) % tc.candidates.len();
            let replacement = tc.candidates[tc.index].clone();
            let suffix = if tc.start == 0 { ": " } else { " " };
            // Everything after cursor is preserved; replace start..cursor
            let before = self.input.text[..tc.start].to_string();
            let after = self.input.text[self.input.cursor..].to_string();
            self.input.text = format!("{before}{replacement}{suffix}{after}");
            self.input.cursor = tc.start + replacement.len() + suffix.len();
        } else {
            // Start new completion — find the word at cursor
            let before_cursor = &self.input.text[..self.input.cursor];
            let word_start = before_cursor.rfind(' ').map(|i| i + 1).unwrap_or(0);
            let prefix = before_cursor[word_start..].to_string();

            if prefix.is_empty() {
                return;
            }

            let prefix_lower = prefix.to_lowercase();
            let candidates: Vec<String> = self
                .known_users
                .iter()
                .filter(|nick| nick.to_lowercase().starts_with(&prefix_lower))
                .cloned()
                .collect();

            if candidates.is_empty() {
                return;
            }

            let replacement = candidates[0].clone();
            let after = self.input.text[self.input.cursor..].to_string();
            let before = self.input.text[..word_start].to_string();
            let suffix = if word_start == 0 { ": " } else { " " };
            self.input.text = format!("{before}{replacement}{suffix}{after}");
            self.input.cursor = word_start + replacement.len() + suffix.len();

            self.tab_completion = Some(TabCompletion {
                prefix,
                start: word_start,
                candidates,
                index: 0,
            });
        }
    }

    /// Reset tab completion state (call on any non-Tab input)
    pub fn reset_tab_completion(&mut self) {
        self.tab_completion = None;
    }

    pub fn scroll_to_bottom(&mut self) {
        if !self.messages.is_empty() {
            self.scroll_position = self.messages.len().saturating_sub(1);
            self.scroll_state = self.scroll_state.position(self.scroll_position);
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_position = self.scroll_position.saturating_sub(amount);
        self.scroll_state = self.scroll_state.position(self.scroll_position);
        self.pinned_to_bottom = false;
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let max = self.messages.len().saturating_sub(1);
        self.scroll_position = (self.scroll_position + amount).min(max);
        self.scroll_state = self.scroll_state.position(self.scroll_position);

        // Re-pin if we've scrolled back to the bottom
        if self.scroll_position >= max {
            self.pinned_to_bottom = true;
        }
    }

    pub fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                // Save current input and start browsing
                self.history_stash = self.input.text.clone();
                self.history_index = Some(self.input_history.len() - 1);
                let text = self.input_history.last().unwrap().clone();
                self.input.set(text);
            }
            Some(0) => {
                // Already at oldest entry
            }
            Some(i) => {
                self.history_index = Some(i - 1);
                let text = self.input_history[i - 1].clone();
                self.input.set(text);
            }
        }
    }

    pub fn history_next(&mut self) {
        match self.history_index {
            None => {}
            Some(i) => {
                if i + 1 >= self.input_history.len() {
                    // Back to current input
                    self.history_index = None;
                    let stash = self.history_stash.clone();
                    self.input.set(stash);
                } else {
                    self.history_index = Some(i + 1);
                    let text = self.input_history[i + 1].clone();
                    self.input.set(text);
                }
            }
        }
    }

    pub async fn refresh_messages(&mut self) -> Result<()> {
        let current_count = self.messages.len();
        self.load_messages().await?;

        if self.messages.len() > current_count {
            self.needs_bell = true;
            if self.pinned_to_bottom {
                self.scroll_to_bottom();
            }
        }

        Ok(())
    }

    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    /// Number of new messages below the current scroll view
    pub fn unread_below(&self, visible_height: usize) -> usize {
        if self.pinned_to_bottom || self.messages.is_empty() {
            return 0;
        }
        let visible_end = (self.scroll_position + visible_height).min(self.messages.len());
        self.messages.len().saturating_sub(visible_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── InputLine ──────────────────────────────────────────────

    #[test]
    fn input_insert_and_cursor() {
        let mut input = InputLine::new();
        input.insert('h');
        input.insert('i');
        assert_eq!(input.text, "hi");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn input_backspace() {
        let mut input = InputLine::new();
        input.set("hello".into());
        input.backspace();
        assert_eq!(input.text, "hell");
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn input_backspace_at_start() {
        let mut input = InputLine::new();
        input.set("hello".into());
        input.home();
        input.backspace(); // should be no-op
        assert_eq!(input.text, "hello");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_delete() {
        let mut input = InputLine::new();
        input.set("hello".into());
        input.home();
        input.delete();
        assert_eq!(input.text, "ello");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_delete_at_end() {
        let mut input = InputLine::new();
        input.set("hello".into());
        input.delete(); // should be no-op
        assert_eq!(input.text, "hello");
    }

    #[test]
    fn input_cursor_movement() {
        let mut input = InputLine::new();
        input.set("hello".into());
        assert_eq!(input.cursor, 5);

        input.move_left();
        assert_eq!(input.cursor, 4);

        input.home();
        assert_eq!(input.cursor, 0);

        input.move_right();
        assert_eq!(input.cursor, 1);

        input.end();
        assert_eq!(input.cursor, 5);

        // Boundary: left at 0
        input.home();
        input.move_left();
        assert_eq!(input.cursor, 0);

        // Boundary: right at end
        input.end();
        input.move_right();
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn input_insert_mid_string() {
        let mut input = InputLine::new();
        input.set("hllo".into());
        input.home();
        input.move_right(); // cursor at 1
        input.insert('e');
        assert_eq!(input.text, "hello");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn input_word_movement() {
        let mut input = InputLine::new();
        input.set("hello world foo".into());

        input.move_word_left();
        assert_eq!(input.cursor, 12); // before "foo"

        input.move_word_left();
        assert_eq!(input.cursor, 6); // before "world"

        input.move_word_left();
        assert_eq!(input.cursor, 0); // before "hello"

        input.move_word_left();
        assert_eq!(input.cursor, 0); // stays at 0

        input.move_word_right();
        assert_eq!(input.cursor, 6); // after "hello "

        input.move_word_right();
        assert_eq!(input.cursor, 12); // after "world "

        input.move_word_right();
        assert_eq!(input.cursor, 15); // end
    }

    #[test]
    fn input_kill_to_end() {
        let mut input = InputLine::new();
        input.set("hello world".into());
        input.home();
        input.move_word_right(); // cursor at 6
        input.kill_to_end();
        assert_eq!(input.text, "hello ");
        assert_eq!(input.cursor, 6);
    }

    #[test]
    fn input_kill_to_start() {
        let mut input = InputLine::new();
        input.set("hello world".into());
        input.home();
        input.move_word_right(); // cursor at 6
        input.kill_to_start();
        assert_eq!(input.text, "world");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_kill_word_back() {
        let mut input = InputLine::new();
        input.set("hello world".into());
        input.kill_word_back();
        assert_eq!(input.text, "hello ");
        assert_eq!(input.cursor, 6);
    }

    #[test]
    fn input_utf8() {
        let mut input = InputLine::new();
        input.insert('é');
        input.insert('ñ');
        assert_eq!(input.text, "éñ");
        assert_eq!(input.cursor_chars(), 2);

        input.move_left();
        assert_eq!(input.cursor_chars(), 1);

        input.backspace();
        assert_eq!(input.text, "ñ");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_clear() {
        let mut input = InputLine::new();
        input.set("hello".into());
        input.clear();
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
        assert!(input.is_empty());
    }

    #[test]
    fn input_is_empty_whitespace() {
        let mut input = InputLine::new();
        input.set("   ".into());
        assert!(input.is_empty());
    }

    // ── ChatMessage ────────────────────────────────────────────

    #[test]
    fn message_action() {
        let msg = ChatMessage::new("*".to_string(), "waves".to_string());
        assert!(msg.is_action());

        let msg = ChatMessage::new("alice".to_string(), "hello".to_string());
        assert!(!msg.is_action());
    }
}
