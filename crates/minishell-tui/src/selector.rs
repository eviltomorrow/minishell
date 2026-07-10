use crossterm::event::{self, Event, KeyCode, KeyEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Terminal;
use minishell_core::Machine;

pub struct SelectorState {
    machines: Vec<Machine>,
    cursor: usize,
    selected: Option<Machine>,
    quitting: bool,
}

pub fn select_machine(machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    if machines.is_empty() {
        return Ok(None);
    }
    if machines.len() == 1 {
        return Ok(Some(machines.into_iter().next().unwrap()));
    }

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

    let result = select_inner(&mut terminal, machines);

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen);

    result
}

fn select_inner(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    let mut state = SelectorState {
        machines,
        cursor: 0,
        selected: None,
        quitting: false,
    };

    loop {
        terminal.draw(|f| draw_selector(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            handle_selector_key(&mut state, key);
        }

        if state.quitting || state.selected.is_some() {
            break;
        }
    }

    Ok(state.selected)
}

fn draw_selector(f: &mut ratatui::Frame, state: &mut SelectorState) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    // Title
    let title = Line::from(vec![
        Span::styled(" Select Machine ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(title, chunks[0]);

    // Table header
    let header = Row::new(vec![
        Cell::from(">"),
        Cell::from("#"),
        Cell::from("IP"),
        Cell::from("NAT-IP"),
        Cell::from("Port"),
        Cell::from("User"),
        Cell::from("Remark"),
    ]).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = state.machines.iter().enumerate().map(|(i, m)| {
        let indicator = if i == state.cursor { "▸" } else { " " };
        Row::new(vec![
            Cell::from(indicator),
            Cell::from(format!("{}", i + 1)),
            Cell::from(m.ip.clone()),
            Cell::from(m.nat_ip.clone()),
            Cell::from(format!("{}", m.port)),
            Cell::from(m.username.clone()),
            Cell::from(m.remark.clone()),
        ])
    }).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(20),
        ],
    ).header(header);

    f.render_widget(table, chunks[0]);

    // Help
    let help = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" navigate  ", Style::default().fg(Color::Gray)),
        Span::styled("↵", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" select  ", Style::default().fg(Color::Gray)),
        Span::styled("q", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" quit", Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(help, chunks[1]);
}

fn handle_selector_key(state: &mut SelectorState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => state.quitting = true,
        KeyCode::Up | KeyCode::Char('k') => {
            if state.cursor > 0 {
                state.cursor -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.cursor < state.machines.len() - 1 {
                state.cursor += 1;
            }
        }
        KeyCode::Enter => {
            state.selected = Some(state.machines[state.cursor].clone());
        }
        _ => {}
    }
}
