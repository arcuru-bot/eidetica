use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

pub async fn handle_key_event(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);

    // Help overlay toggle
    if app.show_help {
        app.show_help = false;
        return;
    }

    // Command completion popup intercepts some keys
    if app.has_cmd_completions() {
        match key {
            KeyCode::Up => {
                app.cmd_completion_up();
                return;
            }
            KeyCode::Down => {
                app.cmd_completion_down();
                return;
            }
            KeyCode::Tab | KeyCode::Enter => {
                app.accept_cmd_completion();
                return;
            }
            KeyCode::Esc => {
                app.cmd_completions.clear();
                app.cmd_selected = 0;
                return;
            }
            _ => {} // fall through to normal handling
        }
    }

    // Reset tab completion on any key except Tab
    if key != KeyCode::Tab {
        app.reset_tab_completion();
    }

    match key {
        // Quit: Ctrl+C or Ctrl+Q
        KeyCode::Char('c') | KeyCode::Char('q') if ctrl => {
            app.should_quit = true;
        }

        // Room switching: Alt+1..9
        KeyCode::Char(c @ '1'..='9') if alt => {
            let idx = (c as usize) - ('1' as usize);
            app.switch_room(idx);
        }

        // Room switching: Ctrl+N/P
        KeyCode::Char('n') if ctrl => app.next_room(),
        KeyCode::Char('p') if ctrl => app.prev_room(),

        // Send message
        KeyCode::Enter => {
            app.clear_status_message();
            if let Err(e) = app.send_message().await {
                app.status_message = Some(format!("Error: {e}"));
            }
        }

        // Input editing
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left if ctrl || alt => app.input.move_word_left(),
        KeyCode::Right if ctrl || alt => app.input.move_word_right(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),

        // Emacs-style line editing
        KeyCode::Char('a') if ctrl => app.input.home(),
        KeyCode::Char('e') if ctrl => app.input.end(),
        KeyCode::Char('k') if ctrl => app.input.kill_to_end(),
        KeyCode::Char('u') if ctrl => app.input.kill_to_start(),
        KeyCode::Char('w') if ctrl => app.input.kill_word_back(),

        // Scrolling
        KeyCode::PageUp => app.scroll_up(20),
        KeyCode::PageDown => app.scroll_down(20),

        // History / scroll with Up/Down
        KeyCode::Up if ctrl => app.scroll_up(1),
        KeyCode::Down if ctrl => app.scroll_down(1),
        KeyCode::Up => app.history_prev(),
        KeyCode::Down => app.history_next(),

        // Tab completion
        KeyCode::Tab => app.tab_complete(),

        // Regular character input
        KeyCode::Char(c) => {
            if app.status_message.is_some() {
                app.clear_status_message();
            }
            app.input.insert(c);
        }

        KeyCode::Esc => {}

        _ => {}
    }

    // Update command completions after any input change
    app.update_cmd_completions();
}
