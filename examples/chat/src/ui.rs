use crate::app::App;
use crate::models::ChatMessage;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

pub fn ui(f: &mut ratatui::Frame, app: &App) {
    render_chat(f, app);

    if app.show_help {
        render_help_overlay(f);
    }
}

fn render_chat(f: &mut ratatui::Frame, app: &App) {
    // Main horizontal split: chat area | nick list
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),     // Chat area
            Constraint::Length(20), // Nick list
        ])
        .split(f.area());

    let chat_area = horiz[0];
    let nick_area = horiz[1];

    // Vertical layout for chat area
    let mut constraints = vec![
        Constraint::Length(1), // Topic bar (compact)
    ];

    if app.status_message.is_some() {
        constraints.push(Constraint::Length(1)); // Status line
    }

    constraints.push(Constraint::Min(0)); // Messages
    constraints.push(Constraint::Length(3)); // Input

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(chat_area);

    let mut chunk_idx = 0;

    // Topic bar — compact, no borders
    let room_name = app
        .current_room_name
        .clone()
        .unwrap_or_else(|| "Unknown Room".to_string());

    let mut topic_spans = vec![
        Span::styled(" [", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &room_name,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("] ", Style::default().fg(Color::DarkGray)),
    ];
    if app.room_password.is_some() {
        topic_spans.push(Span::styled(
            "[encrypted] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    topic_spans.push(Span::styled(
        app.current_room_address.as_deref().unwrap_or(""),
        Style::default().fg(Color::DarkGray),
    ));

    let topic_bar =
        Paragraph::new(Line::from(topic_spans)).style(Style::default().bg(Color::Rgb(30, 30, 40)));
    f.render_widget(topic_bar, chunks[chunk_idx]);
    chunk_idx += 1;

    // Status message
    if let Some(status_msg) = &app.status_message {
        let status = Paragraph::new(Span::styled(
            format!(" {status_msg}"),
            Style::default().fg(Color::Yellow),
        ))
        .style(Style::default().bg(Color::Rgb(40, 40, 20)));
        f.render_widget(status, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Messages area
    let msg_chunk = chunks[chunk_idx];
    let inner_width = msg_chunk.width.saturating_sub(2) as usize; // subtract borders
    let inner_height = msg_chunk.height.saturating_sub(2) as usize;
    chunk_idx += 1;

    let lines: Vec<Line> = app
        .messages
        .iter()
        .map(|m| format_message(m, &app.username, app.show_timestamps))
        .collect();

    // Calculate total wrapped lines for scroll math
    let total_wrapped = lines
        .iter()
        .map(|line| wrapped_line_count(line, inner_width).max(1))
        .sum::<usize>();

    // Scroll position: show the bottom when pinned, otherwise use scroll_position
    let scroll_row = if app.pinned_to_bottom {
        total_wrapped.saturating_sub(inner_height)
    } else {
        // Convert message-index scroll position to wrapped-line position
        let mut row = 0usize;
        for line in lines.iter().take(app.scroll_position) {
            row += wrapped_line_count(line, inner_width).max(1);
        }
        row
    };

    // Unread: count wrapped lines below the viewport
    let visible_end = scroll_row + inner_height;
    let unread_lines = total_wrapped.saturating_sub(visible_end);
    let title = if unread_lines > 0 && !app.pinned_to_bottom {
        format!(" Messages ({}) -- more below \u{2193} ", app.messages.len())
    } else {
        format!(" Messages ({}) ", app.messages.len())
    };

    let messages_paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(
                    Style::default().fg(if unread_lines > 0 && !app.pinned_to_bottom {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    }),
                )
                .title(title),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_row as u16, 0));

    f.render_widget(messages_paragraph, msg_chunk);

    // Input area
    let input_chunk = chunks[chunk_idx];

    let input_title = if let Some(idx) = app.history_index {
        format!(" [{}/{}] ", idx + 1, app.input_history.len())
    } else {
        " > ".to_string()
    };

    let input = Paragraph::new(app.input.text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(input_title),
        );
    f.render_widget(input, input_chunk);

    // Cursor position (byte-aware)
    f.set_cursor_position((
        input_chunk.x + app.input.cursor_chars() as u16 + 1,
        input_chunk.y + 1,
    ));

    // Nick list
    render_nick_list(f, app, nick_area);
}

fn render_nick_list(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let nicks: Vec<ListItem> = app
        .known_users
        .iter()
        .map(|nick| {
            let style = if nick == &app.username {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(nick_color(nick))
            };
            ListItem::new(Span::styled(format!(" {nick}"), style))
        })
        .collect();

    let nick_list = List::new(nicks).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" Users ({}) ", app.known_users.len())),
    );

    f.render_widget(nick_list, area);
}

fn render_help_overlay(f: &mut ratatui::Frame) {
    let area = centered_rect(60, 70, f.area());

    f.render_widget(Clear, area);

    let help_text = vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        help_line("Ctrl+C / Ctrl+Q", "Quit"),
        help_line("Enter", "Send message"),
        help_line("Up / Down", "Input history"),
        help_line("Ctrl+Up / Ctrl+Down", "Scroll messages"),
        help_line("PageUp / PageDown", "Scroll messages (fast)"),
        help_line("Left / Right", "Move cursor"),
        help_line("Ctrl+Left / Ctrl+Right", "Move by word"),
        help_line("Home / Ctrl+A", "Start of line"),
        help_line("End / Ctrl+E", "End of line"),
        help_line("Ctrl+K", "Kill to end of line"),
        help_line("Ctrl+U", "Kill to start of line"),
        help_line("Ctrl+W", "Kill word back"),
        help_line("Tab", "Nick completion (cycle)"),
        Line::from(""),
        Line::from(Span::styled(
            "Commands",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        help_line("/nick <name>", "Change nickname"),
        help_line("/me <action>", "Send action message"),
        help_line("/clear", "Clear message display"),
        help_line("/topic", "Show room topic"),
        help_line("/users", "List known users"),
        help_line("/timestamps", "Toggle timestamps"),
        help_line("/encrypt <pass>", "Encrypt room"),
        help_line("/decrypt <pass>", "Unlock encrypted room"),
        help_line("/quit", "Quit"),
        help_line("/help", "Toggle this help"),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Help ")
                .style(Style::default().bg(Color::Rgb(20, 20, 30))),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(help, area);
}

fn help_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {key:<28}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(desc),
    ])
}

/// Format a chat message into a styled Line
fn format_message<'a>(m: &'a ChatMessage, username: &str, show_timestamps: bool) -> Line<'a> {
    let mut spans = Vec::new();

    if show_timestamps {
        let timestamp = m.timestamp.format("%H:%M:%S");
        spans.push(Span::styled(
            format!("[{timestamp}] "),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if m.is_action() {
        let action_text = format!("* {}", m.content);
        let base_style = Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::ITALIC);
        spans.extend(style_content(&action_text, username, base_style));
    } else {
        let is_own = m.author == username;
        let color = if is_own {
            Color::White
        } else {
            nick_color(&m.author)
        };

        spans.push(Span::styled(
            format!("{}: ", m.author),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        let base_style = Style::default();
        spans.extend(style_content(&m.content, username, base_style));
    }

    Line::from(spans)
}

/// Apply both mention highlighting and URL detection to text
fn style_content(text: &str, username: &str, base_style: Style) -> Vec<Span<'static>> {
    let mention_spans = highlight_mentions(text, username, base_style);
    // Apply URL highlighting within each span
    let mut result = Vec::new();
    for span in mention_spans {
        result.extend(highlight_urls(&span.content, span.style));
    }
    result
}

/// Split text into spans, highlighting occurrences of `username` with a bold yellow style
fn highlight_mentions(text: &str, username: &str, base_style: Style) -> Vec<Span<'static>> {
    if username.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let mention_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let lower_text = text.to_lowercase();
    let lower_nick = username.to_lowercase();
    let mut spans = Vec::new();
    let mut last = 0;

    for (start, _) in lower_text.match_indices(&lower_nick) {
        // Check word boundaries: mention should not be part of a larger word
        let before_ok = start == 0 || !text.as_bytes()[start - 1].is_ascii_alphanumeric();
        let end = start + username.len();
        let after_ok = end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric();

        if before_ok && after_ok {
            if start > last {
                spans.push(Span::styled(text[last..start].to_string(), base_style));
            }
            spans.push(Span::styled(text[start..end].to_string(), mention_style));
            last = end;
        }
    }

    if last < text.len() {
        spans.push(Span::styled(text[last..].to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

/// Split text into spans, highlighting URLs with underlined blue
fn highlight_urls(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let url_style = Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::UNDERLINED);

    let mut spans = Vec::new();
    let mut last = 0;

    // Simple URL detection: find http:// or https:// and extend to whitespace
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if rest.starts_with("http://") || rest.starts_with("https://") {
            // Found URL start — extend to next whitespace or end
            let url_end = rest
                .find(|c: char| c.is_whitespace())
                .map(|j| i + j)
                .unwrap_or(text.len());

            if i > last {
                spans.push(Span::styled(text[last..i].to_string(), base_style));
            }
            spans.push(Span::styled(text[i..url_end].to_string(), url_style));
            last = url_end;
            i = url_end;
        } else {
            i += 1;
        }
    }

    if last < text.len() {
        spans.push(Span::styled(text[last..].to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

/// Estimate the number of wrapped lines a Line will occupy at a given width
fn wrapped_line_count(line: &Line, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let total_chars: usize = line.spans.iter().map(|s| s.content.len()).sum();
    if total_chars == 0 {
        return 1;
    }
    (total_chars + width - 1) / width
}

/// Generate a consistent color for a nick
fn nick_color(nick: &str) -> Color {
    let colors = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
    ];
    let hash: u32 = nick
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    colors[(hash as usize) % colors.len()]
}

/// Create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_texts<'a>(spans: &'a [Span]) -> Vec<&'a str> {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn highlight_mentions_basic() {
        let base = Style::default();
        let spans = highlight_mentions("hey alice how are you", "alice", base);
        assert_eq!(span_texts(&spans), vec!["hey ", "alice", " how are you"]);
    }

    #[test]
    fn highlight_mentions_case_insensitive() {
        let base = Style::default();
        let spans = highlight_mentions("hey Alice!", "alice", base);
        assert_eq!(span_texts(&spans), vec!["hey ", "Alice", "!"]);
    }

    #[test]
    fn highlight_mentions_no_match() {
        let base = Style::default();
        let spans = highlight_mentions("hey bob", "alice", base);
        assert_eq!(span_texts(&spans), vec!["hey bob"]);
    }

    #[test]
    fn highlight_mentions_word_boundary() {
        let base = Style::default();
        // "alice" inside "malice" should NOT match
        let spans = highlight_mentions("malice aforethought", "alice", base);
        assert_eq!(span_texts(&spans), vec!["malice aforethought"]);
    }

    #[test]
    fn highlight_mentions_at_start() {
        let base = Style::default();
        let spans = highlight_mentions("alice: hello", "alice", base);
        assert_eq!(span_texts(&spans), vec!["alice", ": hello"]);
    }

    #[test]
    fn highlight_mentions_at_end() {
        let base = Style::default();
        let spans = highlight_mentions("hello alice", "alice", base);
        assert_eq!(span_texts(&spans), vec!["hello ", "alice"]);
    }

    #[test]
    fn highlight_mentions_multiple() {
        let base = Style::default();
        let spans = highlight_mentions("alice and alice again", "alice", base);
        assert_eq!(
            span_texts(&spans),
            vec!["alice", " and ", "alice", " again"]
        );
    }

    #[test]
    fn nick_color_consistent() {
        let c1 = nick_color("alice");
        let c2 = nick_color("alice");
        assert_eq!(c1, c2);

        // Different nicks should (usually) get different colors
        let c3 = nick_color("bob");
        // Not guaranteed different, but should be for these short strings
        assert_ne!(c1, c3);
    }

    #[test]
    fn wrapped_line_count_basic() {
        let line = Line::from("hello world"); // 11 chars
        assert_eq!(wrapped_line_count(&line, 80), 1);
        assert_eq!(wrapped_line_count(&line, 5), 3); // 11/5 = 3
        assert_eq!(wrapped_line_count(&line, 11), 1);
    }

    #[test]
    fn wrapped_line_count_empty() {
        let line = Line::from("");
        assert_eq!(wrapped_line_count(&line, 80), 1); // empty = 1 line
    }

    // ── URL highlighting ──────────────────────────────────────

    #[test]
    fn highlight_urls_basic() {
        let base = Style::default();
        let spans = highlight_urls("check https://example.com for info", base);
        assert_eq!(
            span_texts(&spans),
            vec!["check ", "https://example.com", " for info"]
        );
    }

    #[test]
    fn highlight_urls_http() {
        let base = Style::default();
        let spans = highlight_urls("see http://foo.bar/baz", base);
        assert_eq!(span_texts(&spans), vec!["see ", "http://foo.bar/baz"]);
    }

    #[test]
    fn highlight_urls_no_url() {
        let base = Style::default();
        let spans = highlight_urls("no urls here", base);
        assert_eq!(span_texts(&spans), vec!["no urls here"]);
    }

    #[test]
    fn highlight_urls_multiple() {
        let base = Style::default();
        let spans = highlight_urls("visit https://a.com and https://b.com today", base);
        assert_eq!(
            span_texts(&spans),
            vec![
                "visit ",
                "https://a.com",
                " and ",
                "https://b.com",
                " today"
            ]
        );
    }

    #[test]
    fn highlight_urls_at_start() {
        let base = Style::default();
        let spans = highlight_urls("https://start.com is cool", base);
        assert_eq!(span_texts(&spans), vec!["https://start.com", " is cool"]);
    }
}
