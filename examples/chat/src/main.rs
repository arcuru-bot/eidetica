mod app;
mod handlers;
mod models;
mod ui;

use app::App;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use eidetica::{Instance, Result, backend::database::Sqlite};
use handlers::handle_key_event;
use models::ChatMessage;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::PathBuf;
use ui::ui;

#[derive(Parser)]
#[command(name = "eidetica-chat")]
#[command(about = "A chat application using Eidetica for distributed messaging")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Username for the chat session
    #[arg(short, long, global = true)]
    username: Option<String>,

    /// Enable verbose debug output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Transport to use for sync (http or iroh)
    #[arg(long, default_value = "iroh", global = true)]
    transport: String,

    /// Directory for persistent data (SQLite database).
    /// Defaults to ~/.local/share/eidetica-chat
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new chat room and print the ticket
    Create {
        /// Room name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Send a message to a chat room
    Send {
        /// Ticket URL of the room
        ticket: String,

        /// Message text to send
        message: String,
    },

    /// List messages from a chat room
    Messages {
        /// Ticket URL of the room
        ticket: String,

        /// Continuously watch for new messages
        #[arg(short, long)]
        follow: bool,

        /// Maximum number of messages to show (0 = all)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Open the interactive TUI chat
    Tui {
        /// Ticket URL to connect to. If not provided, creates a new room.
        ticket: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        tracing_subscriber::fmt().with_env_filter("debug").init();
    } else {
        tracing_subscriber::fmt().with_env_filter("error").init();
    }

    let transport = match cli.transport.to_lowercase().as_str() {
        "http" => "http",
        "iroh" => "iroh",
        _ => {
            eprintln!(
                "Invalid transport '{}'. Use 'http' or 'iroh'",
                cli.transport
            );
            std::process::exit(1);
        }
    };

    let username = cli
        .username
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "Anonymous".to_string());

    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);

    match cli.command {
        Some(Command::Create { name }) => cmd_create(&username, transport, &data_dir, name).await,
        Some(Command::Send { ticket, message }) => {
            cmd_send(&username, transport, &data_dir, &ticket, &message).await
        }
        Some(Command::Messages {
            ticket,
            follow,
            limit,
            json,
        }) => {
            cmd_messages(
                &username, transport, &data_dir, &ticket, follow, limit, json,
            )
            .await
        }
        Some(Command::Tui { ticket }) => cmd_tui(&username, transport, &data_dir, ticket).await,
        None => cmd_tui(&username, transport, &data_dir, None).await,
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("eidetica-chat")
}

/// Create a new room and print the ticket URL
async fn cmd_create(
    username: &str,
    transport: &str,
    data_dir: &PathBuf,
    name: Option<String>,
) -> Result<()> {
    let (mut app, _) = setup_app(username, transport, data_dir).await?;

    let room_name = name.unwrap_or_else(|| {
        format!(
            "Chat Room - {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
        )
    });

    app.create_room(&room_name).await?;

    if let Some(addr) = &app.current_room_address {
        // Print ticket to stdout for scripting; info to stderr
        eprintln!("Room created: {room_name}");
        println!("{addr}");
    }

    Ok(())
}

/// Send a single message to a room
async fn cmd_send(
    username: &str,
    transport: &str,
    data_dir: &PathBuf,
    ticket: &str,
    message: &str,
) -> Result<()> {
    let (mut app, _) = setup_app(username, transport, data_dir).await?;
    let local = app.open_room(ticket).await?;

    let msg = ChatMessage::new(username.to_string(), message.to_string());

    if let Some(database) = &app.current_room {
        let txn = database.new_transaction().await?;
        let store = txn
            .get_store::<eidetica::store::Table<ChatMessage>>("messages")
            .await?;
        store.insert(msg).await?;
        txn.commit().await?;
    }

    if !local {
        // Give sync a moment to propagate to remote peers
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    eprintln!("Message sent.");
    Ok(())
}

/// List messages from a room
async fn cmd_messages(
    username: &str,
    transport: &str,
    data_dir: &PathBuf,
    ticket: &str,
    follow: bool,
    limit: usize,
    json: bool,
) -> Result<()> {
    let (mut app, _) = setup_app(username, transport, data_dir).await?;
    app.open_room(ticket).await?;

    let msgs = if limit == 0 || limit >= app.messages.len() {
        &app.messages[..]
    } else {
        &app.messages[app.messages.len() - limit..]
    };

    for msg in msgs {
        print_message(msg, json);
    }

    if follow {
        let mut seen = app.messages.len();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            app.load_messages().await?;
            if app.messages.len() > seen {
                for msg in &app.messages[seen..] {
                    print_message(msg, json);
                }
                seen = app.messages.len();
            }
        }
    }

    Ok(())
}

fn print_message(msg: &ChatMessage, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(msg).unwrap_or_else(|_| "{}".to_string())
        );
    } else if msg.is_action() {
        println!("[{}] * {}", msg.timestamp.format("%H:%M:%S"), msg.content);
    } else {
        println!(
            "[{}] {}: {}",
            msg.timestamp.format("%H:%M:%S"),
            msg.author,
            msg.content
        );
    }
}

/// Run the interactive TUI
async fn cmd_tui(
    username: &str,
    transport: &str,
    data_dir: &PathBuf,
    ticket: Option<String>,
) -> Result<()> {
    let (mut app, _) = setup_app(username, transport, data_dir).await?;

    if let Some(ticket) = ticket {
        eprintln!("Connecting to room...");
        app.connect_to_room(&ticket).await?;
        eprintln!("Connected!");
    } else {
        let room_name = format!(
            "Chat Room - {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
        );
        app.create_room(&room_name).await?;

        if let Some(addr) = &app.current_room_address {
            eprintln!("Eidetica Chat Room Created!");
            eprintln!();
            eprintln!("Room Address: {addr}");
            eprintln!("Username: {username}");
            eprintln!();
            eprintln!("Share this address with others to invite them.");
            eprintln!();
            eprintln!("Press Enter to start chatting...");

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("{err:?}");
    }

    Ok(())
}

/// Shared setup: create instance + app
async fn setup_app(username: &str, transport: &str, data_dir: &PathBuf) -> Result<(App, String)> {
    // Ensure data directory exists
    std::fs::create_dir_all(data_dir).map_err(|e| {
        eidetica::sync::SyncError::Network(format!(
            "Failed to create data directory {}: {e}",
            data_dir.display()
        ))
    })?;

    let db_path = data_dir.join("chat.db");
    let backend = Sqlite::open(&db_path).await?;
    let instance = Instance::open(Box::new(backend)).await?;
    instance.enable_sync().await?;

    let _ = instance.create_user(username, None).await;
    let user = instance.login_user(username, None).await?;

    let app = App::new(instance, user, username.to_string(), transport)?;
    Ok((app, username.to_string()))
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let mut refresh_interval = tokio::time::interval(tokio::time::Duration::from_secs(2));

    loop {
        terminal.draw(|f| ui(f, app))?;

        // Ring bell and update title on new messages
        if app.needs_bell {
            app.needs_bell = false;
            execute!(
                terminal.backend_mut(),
                crossterm::terminal::SetTitle(format!(
                    "* eidetica-chat ({})",
                    app.current_room_name.as_deref().unwrap_or("chat")
                ))
            )?;
            // Terminal bell
            print!("\x07");
        }

        let mut handled_event = false;

        while event::poll(std::time::Duration::from_millis(0))? {
            if let Ok(Event::Key(key)) = event::read() {
                handled_event = true;
                if key.kind == KeyEventKind::Press {
                    handle_key_event(app, key.code, key.modifiers).await;
                }
            }
        }

        if !handled_event {
            tokio::select! {
                _ = refresh_interval.tick() => {
                    if let Err(e) = app.refresh_messages().await {
                        eprintln!("Error refreshing messages: {e}");
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
