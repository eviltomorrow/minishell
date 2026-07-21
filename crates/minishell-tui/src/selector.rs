use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use minishell_core::Machine;
use minishell_utils::pad_right;

struct Col {
    title: &'static str,
    width: usize,
}

const BOLD_CYAN: &str = "\x1b[1;36m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_WHITE: &str = "\x1b[1;97m";
const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";

const COLS: &[Col] = &[
    Col { title: "#", width: 4 },
    Col { title: "IP", width: 15 },
    Col { title: "NAT-IP", width: 12 },
    Col { title: "Port", width: 6 },
    Col { title: "User", width: 10 },
    Col { title: "Remark", width: 20 },
];

fn format_row(idx: usize, m: &Machine) -> String {
    let mut s = pad_right(&format!("{}", idx + 1), COLS[0].width);
    s.push_str(&pad_right(&m.ip, COLS[1].width));
    s.push_str(&pad_right(&m.nat_ip, COLS[2].width));
    s.push_str(&pad_right(&format!("{}", m.port), COLS[3].width));
    s.push_str(&pad_right(&m.username, COLS[4].width));
    s.push_str(&pad_right(&m.remark, COLS[5].width));
    s
}

pub fn select_machine(machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    if machines.is_empty() {
        return Ok(None);
    }
    if machines.len() == 1 {
        return Ok(Some(machines.into_iter().next().unwrap()));
    }

    use std::io::Write;
    let machine_count = machines.len();

    let row_strs: Vec<String> = machines.iter().enumerate()
        .map(|(i, m)| format_row(i, m))
        .collect();

    // Print everything inline — no clearing
    println!();
    println!(
        "  {BOLD_CYAN}选择要登录的机器{RESET} {BOLD_GREEN}(↑↓{RESET}{DIM} 导航{RESET} \
         {BOLD_GREEN}Enter{RESET}{DIM} 选择{RESET} {BOLD_GREEN}Esc{RESET}{DIM} 取消){RESET}"
    );
    println!();

    let header: String = COLS.iter().map(|c| format!("{BOLD_CYAN}{}{RESET}", pad_right(c.title, c.width))).collect();
    println!("  {}", header);

    let (term_w, _) = crossterm::terminal::size().unwrap_or((80, 24));
    println!("{DIM}{}{RESET}", "─".repeat(term_w as usize));

    for (i, row) in row_strs.iter().enumerate() {
        let line = if i == 0 {
            format!("  {BOLD_WHITE}▸{RESET}{}", row)
        } else {
            format!("  {DIM} {RESET}{}", row)
        };
        if i == machine_count - 1 {
            print!("{}", line);
        } else {
            println!("{}", line);
        }
    }

    let _ = std::io::stdout().flush();
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide)?;
    crossterm::terminal::enable_raw_mode()?;

    let mut cursor = 0usize;

    let result = loop {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    print!("\r\n");
                    let _ = std::io::stdout().flush();
                    break None;
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    print!("\r\n");
                    let _ = std::io::stdout().flush();
                    break None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if cursor > 0 {
                        let old = cursor;
                        cursor -= 1;
                        redraw_row(machine_count, old, &row_strs[old], false);
                        redraw_row(machine_count, cursor, &row_strs[cursor], true);
                        let _ = std::io::stdout().flush();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if cursor < machine_count - 1 {
                        let old = cursor;
                        cursor += 1;
                        redraw_row(machine_count, old, &row_strs[old], false);
                        redraw_row(machine_count, cursor, &row_strs[cursor], true);
                        let _ = std::io::stdout().flush();
                    }
                }
                KeyCode::Enter => {
                    print!("\r\n");
                    let _ = std::io::stdout().flush();
                    break Some(machines[cursor].clone());
                }
                _ => {}
            }
        }
    };

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
    Ok(result)
}

fn redraw_row(machine_count: usize, idx: usize, text: &str, selected: bool) {
    let indicator = if selected { "▸" } else { " " };
    let color = if selected { BOLD_WHITE } else { DIM };
    let up = machine_count - 1 - idx;
    if up > 0 {
        print!("\x1b[{}A", up);
    }
    print!("\r\x1b[2K  {}{}{}{}", color, indicator, RESET, text);
    if up > 0 {
        print!("\x1b[{}B", up);
    }
}
