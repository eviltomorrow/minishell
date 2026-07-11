use std::sync::Arc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use minishell_core::Machine;
use minishell_store::Store;

use super::form::{FormState, DeleteState};
use super::table::{MachineTable, default_columns, secrets_columns, format_machine_row};
use super::styles;

pub struct AppState {
    pub store: Arc<Store>,
    pub machines: Vec<Machine>,
    pub table: MachineTable,
    pub search_input: String,
    pub search_focused: bool,
    pub show_secrets: bool,
    pub form: Option<FormState>,
    pub delete_confirm: Option<DeleteState>,
    pub should_quit: bool,
    pub login_target: Option<Machine>,
}

pub fn run(store: Arc<Store>) -> anyhow::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        default_hook(info);
    }));

    let result = run_inner(&mut terminal, store);

    result
}

fn run_inner(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, store: Arc<Store>) -> anyhow::Result<()> {
    let machines = store.search("")?;
    let _total = store.count_all()?;
    let columns = default_columns();
    let table = MachineTable::new(columns);

    let mut state = AppState {
        store,
        machines,
        table,
        search_input: String::new(),
        search_focused: false,
        show_secrets: false,
        form: None,
        delete_confirm: None,
        should_quit: false,
        login_target: None,
    };

    // Initial table data
    rebuild_table(&mut state);

    loop {
        if let Some(machine) = state.login_target.take() {
            terminal.clear()?;
            let _ = crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen);
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = minishell_ssh::login_to_machine(&machine);
            break;
        }

        terminal.draw(|f| view(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            update(&mut state, key);
        }

        if state.should_quit {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen);
            break;
        }
    }

    Ok(())
}

fn rebuild_table(state: &mut AppState) {
    let rows: Vec<Vec<String>> = state.machines.iter().enumerate()
        .map(|(i, m)| format_machine_row(i, m, state.show_secrets))
        .collect();

    if state.show_secrets {
        state.table.columns = secrets_columns();
    } else {
        state.table.columns = default_columns();
    }
    state.table.set_rows(rows);
}

fn view(f: &mut ratatui::Frame, state: &mut AppState) {
    let area = f.area();

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),   // title
            Constraint::Length(1),   // search bar
            Constraint::Length(1),   // separator
            Constraint::Min(5),     // table
            Constraint::Length(1),   // separator
            Constraint::Length(1),   // status + help combined
        ])
        .split(area);

    // Title
    let title = Line::from(vec![
        Span::styled(" minishell ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} machines", state.machines.len()), Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(title, main_chunks[0]);

    // Search bar
    {
        let search_area = main_chunks[1];
        let search_text = if state.search_focused {
            format!("search> {}▌", state.search_input)
        } else if !state.search_input.is_empty() {
            format!("search> {} (Esc to clear)", state.search_input)
        } else {
            "search> (press / to search)".to_string()
        };
        let style = if state.search_focused { styles::search_style() } else { styles::status_style() };
        let search_para = Paragraph::new(Line::from(vec![
            Span::styled(search_text, style),
        ]));
        f.render_widget(search_para, search_area);
    }

    // Separator
    let sep = Line::from(vec![Span::styled("─".repeat(area.width as usize), styles::separator_style())]);
    f.render_widget(sep, main_chunks[2]);

    // Table
    let table_height = main_chunks[3].height;
    state.table.set_size(area.width, table_height);
    let selected_style = styles::selected_style();
    let normal_style = Style::default();
    state.table.render(main_chunks[3], f.buffer_mut(), selected_style, normal_style);

    // Separator
    let sep2 = Line::from(vec![Span::styled("─".repeat(area.width as usize), styles::separator_style())]);
    f.render_widget(sep2, main_chunks[4]);

    // Status + Help bar
    let mut status_spans: Vec<Span> = vec![];
    if let Some(m) = state.machines.get(state.table.cursor()) {
        status_spans.push(Span::styled(format!("{}/{}", state.table.cursor() + 1, state.machines.len()), styles::status_style()));
        status_spans.push(Span::styled(" │ ", styles::status_sep_style()));
        status_spans.push(Span::styled(m.username.clone(), styles::status_label_style()));
        status_spans.push(Span::styled("@", styles::status_sep_style()));
        status_spans.push(Span::styled(format!("{}:{}", m.effective_host(), m.port), styles::status_style()));
        status_spans.push(Span::styled(" │ ", styles::status_sep_style()));
        status_spans.push(Span::styled("secrets ", styles::status_sep_style()));
        status_spans.push(Span::styled(
            if state.show_secrets { "show" } else { "hide" },
            if state.show_secrets { styles::search_style() } else { styles::status_style() },
        ));
    } else {
        status_spans.push(Span::styled("0/0", styles::status_style()));
        status_spans.push(Span::styled(" │ ", styles::status_sep_style()));
        status_spans.push(Span::styled("no machines", styles::status_sep_style()));
    }

    let help_items: Vec<(&str, &str)> = vec![
        ("↑↓", "sel"),
        ("↵", "login"),
        ("e", "edit"),
        ("a", "add"),
        ("d", "del"),
        ("s", "secrets"),
        ("/", "search"),
        ("q", "quit"),
    ];
    let status_len: usize = status_spans.iter().map(|s| s.width()).sum();
    let help_len: usize = help_items.iter().map(|(k, d)| 1 + k.len() + 1 + d.len() + 1).sum();
    let w = area.width as usize;
    let padding = if w > status_len + help_len + 4 { w - status_len - help_len - 4 } else { 0 };

    status_spans.push(Span::raw(" ".repeat(padding)));
    for (key, desc) in &help_items {
        status_spans.push(Span::styled(format!(" {} ", key), styles::status_key_style()));
        status_spans.push(Span::styled(format!("{} ", desc), styles::status_desc_style()));
    }

    let bottom_line = Line::from(status_spans);
    f.render_widget(bottom_line, main_chunks[5]);

    // Dialog overlays - positioned above status bar with gap
    let gap = 2u16; // gap between dialog and status bar
    if let Some(ref form_state) = state.form {
        let status_y = main_chunks[5].y;
        let dialog_height = 12u16;
        let dialog_width = (area.width * 50 / 100).min(area.width);
        let dialog_area = Rect {
            x: 0,
            y: status_y.saturating_sub(dialog_height + gap),
            width: dialog_width,
            height: dialog_height,
        };
        f.render_widget(ratatui::widgets::Clear, dialog_area);
        render_form(f, dialog_area, form_state);
    } else if let Some(ref del_state) = state.delete_confirm {
        let status_y = main_chunks[5].y;
        let dialog_height = 6u16;
        let dialog_width = (area.width * 40 / 100).min(area.width);
        let dialog_area = Rect {
            x: 0,
            y: status_y.saturating_sub(dialog_height + gap),
            width: dialog_width,
            height: dialog_height,
        };
        f.render_widget(ratatui::widgets::Clear, dialog_area);
        render_delete_confirm(f, dialog_area, &del_state.target);
    }
}

fn render_form(f: &mut ratatui::Frame, area: Rect, form_state: &FormState) {
    let title = if form_state.is_edit { " Edit Machine " } else { " Add Machine " };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(styles::form_box_style())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form_state.fields.iter().enumerate() {
        let style = if i == form_state.step { styles::form_field_style() } else { Style::default() };
        if let Some(ref options) = field.select_options {
            if i == form_state.step {
                let mut spans = vec![Span::styled(format!("{:>12} ", field.label), style)];
                for (j, opt) in options.iter().enumerate() {
                    if j == field.select_index {
                        spans.push(Span::styled(format!(" [{}] ", opt), styles::form_field_style()));
                    } else {
                        spans.push(Span::styled(format!("  {}  ", opt), styles::help_style()));
                    }
                }
                spans.push(Span::styled(" ▌", style));
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:>12} ", field.label), style),
                    Span::styled(&field.value, style),
                ]));
            }
        } else {
            let cursor = if i == form_state.step { "▌" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("{:>12} ", field.label), style),
                Span::styled(format!("{}{}", field.value, cursor), style),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" ↑↓", styles::key_style()),
        Span::styled(" next  ", styles::help_style()),
        Span::styled("↵", styles::key_style()),
        Span::styled(" save  ", styles::help_style()),
        Span::styled("Esc", styles::key_style()),
        Span::styled(" back", styles::help_style()),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn render_delete_confirm(f: &mut ratatui::Frame, area: Rect, target: &Machine) {
    let block = Block::default()
        .title("Delete Machine")
        .borders(Borders::ALL)
        .border_style(styles::delete_box_style())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let msg = format!("Remove {} ({})?", target.ip, target.username);
    let line = Line::from(Span::styled(msg, Style::default()));
    f.render_widget(line, Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });

    let hints = Line::from(vec![
        Span::styled("y", styles::key_style()),
        Span::styled(" yes  ", styles::help_style()),
        Span::styled("n", styles::key_style()),
        Span::styled(" no  ", styles::help_style()),
        Span::styled("Esc", styles::key_style()),
        Span::styled(" cancel", styles::help_style()),
    ]);
    f.render_widget(hints, Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 });
}

fn update(state: &mut AppState, key: KeyEvent) {
    // Handle form/dialog first
    if state.form.is_some() {
        handle_form_key(state, key);
        return;
    }
    if state.delete_confirm.is_some() {
        handle_delete_key(state, key);
        return;
    }

    // Search focused
    if state.search_focused {
        match key.code {
            KeyCode::Esc => {
                state.search_focused = false;
                state.search_input.clear();
                reload_machines(state);
            }
            KeyCode::Enter => {
                state.search_focused = false;
                reload_machines(state);
            }
            KeyCode::Backspace => {
                state.search_input.pop();
                reload_machines(state);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.search_focused = false;
                state.table.move_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.search_focused = false;
                state.table.move_down();
            }
            KeyCode::Char(c) => {
                state.search_input.push(c);
                reload_machines(state);
            }
            _ => {}
        }
        return;
    }

    // Normal mode
    match key.code {
        KeyCode::Char('q') => state.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Up | KeyCode::Char('k') => state.table.move_up(),
        KeyCode::Down | KeyCode::Char('j') => state.table.move_down(),
        KeyCode::PageUp => {
            for _ in 0..10 {
                state.table.move_up();
            }
        }
        KeyCode::PageDown => {
            for _ in 0..10 {
                state.table.move_down();
            }
        }
        KeyCode::Char('g') => state.table.goto_top(),
        KeyCode::Char('G') => state.table.goto_bottom(),
        KeyCode::Enter => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.login_target = Some(m);
            }
        }
        KeyCode::Char('/') => {
            state.search_focused = true;
            state.search_input.clear();
        }
        KeyCode::Char('a') => {
            state.form = Some(FormState::new_add());
        }
        KeyCode::Char('e') => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.form = Some(FormState::new_edit(&m));
            }
        }
        KeyCode::Char('d') => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.delete_confirm = Some(DeleteState::new(m));
            }
        }
        KeyCode::Char('s') => {
            state.show_secrets = !state.show_secrets;
            rebuild_table(state);
        }
        _ => {}
    }
}

fn handle_form_key(state: &mut AppState, key: KeyEvent) {
    let form = state.form.as_mut().unwrap();
    match key.code {
        KeyCode::Esc => {
            state.form = None;
        }
        KeyCode::Up | KeyCode::Char('k') => form.navigate_prev(),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => form.navigate_next(),
        KeyCode::Enter => {
            if form.step == form.fields.len() - 1 {
                // Last field - save
                let machine = form.to_machine();
                if form.is_edit {
                    let _ = state.store.update_machine(&machine);
                } else {
                    let _ = state.store.import_machines(&[machine]);
                }
                state.form = None;
                reload_machines(state);
            } else {
                form.navigate_next();
            }
        }
        KeyCode::Left => {
            if form.fields[form.step].select_options.is_some() {
                let field = &mut form.fields[form.step];
                if field.select_index > 0 {
                    field.select_index -= 1;
                    field.value = field.select_options.as_ref().unwrap()[field.select_index].clone();
                }
            } else {
                form.fields[form.step].move_cursor_left();
            }
        }
        KeyCode::Right => {
            if form.fields[form.step].select_options.is_some() {
                let field = &mut form.fields[form.step];
                let len = field.select_options.as_ref().unwrap().len();
                if field.select_index < len - 1 {
                    field.select_index += 1;
                    field.value = field.select_options.as_ref().unwrap()[field.select_index].clone();
                }
            } else {
                form.fields[form.step].move_cursor_right();
            }
        }
        KeyCode::Char(c) => {
            if form.fields[form.step].select_options.is_none() {
                form.fields[form.step].insert_char(c);
            }
        }
        KeyCode::Backspace => {
            if form.fields[form.step].select_options.is_none() {
                form.fields[form.step].delete_char();
            }
        }
        _ => {}
    }
}

fn handle_delete_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(del) = state.delete_confirm.take() {
                let _ = state.store.delete_machine(del.target.id);
                reload_machines(state);
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.delete_confirm = None;
        }
        _ => {}
    }
}

fn reload_machines(state: &mut AppState) {
    state.machines = state.store.search(&state.search_input).unwrap_or_default();
    rebuild_table(state);
}
