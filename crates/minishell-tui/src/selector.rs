use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use minishell_core::Machine;
use unicode_width::UnicodeWidthStr;

struct Col {
    title: &'static str,
    width: usize,
}

const COLS: &[Col] = &[
    Col { title: "#", width: 4 },
    Col { title: "IP", width: 15 },
    Col { title: "NAT-IP", width: 12 },
    Col { title: "Port", width: 6 },
    Col { title: "User", width: 10 },
    Col { title: "Remark", width: 20 },
];

fn pad_str(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        let mut r = String::new();
        let mut cw = 0;
        for c in s.chars() {
            let cw2 = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if cw + cw2 > width { break; }
            r.push(c);
            cw += cw2;
        }
        return r;
    }
    format!("{}{}", s, " ".repeat(width - w))
}

fn format_row(idx: usize, m: &Machine) -> String {
    let parts: Vec<String> = [
        pad_str(&format!("{}", idx + 1), COLS[0].width),
        pad_str(&m.ip, COLS[1].width),
        pad_str(&m.nat_ip, COLS[2].width),
        pad_str(&format!("{}", m.port), COLS[3].width),
        pad_str(&m.username, COLS[4].width),
        pad_str(&m.remark, COLS[5].width),
    ].to_vec();
    parts.join("")
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
        "  \x1b[1;36m选择要登录的机器\x1b[0m \x1b[1;32m(↑↓\x1b[0m\x1b[90m 导航\x1b[0m \
         \x1b[1;32mEnter\x1b[0m\x1b[90m 选择\x1b[0m \x1b[1;32mEsc\x1b[0m\x1b[90m 取消)\x1b[0m"
    );
    println!();

    let bold_cyan = "\x1b[1;36m";
    let reset = "\x1b[0m";
    let header: String = COLS.iter().map(|c| format!("{}{}{}", bold_cyan, pad_str(c.title, c.width), reset)).collect();
    println!("  {}", header);

    let (term_w, _) = crossterm::terminal::size().unwrap_or((80, 24));
    println!("\x1b[90m{}\x1b[0m", "─".repeat(term_w as usize));

    for (i, row) in row_strs.iter().enumerate() {
        let line = if i == 0 {
            format!("  \x1b[1;97m▸\x1b[0m{}", row)
        } else {
            format!("  \x1b[90m \x1b[0m{}", row)
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
    let color = if selected { "\x1b[1;97m" } else { "\x1b[90m" };
    let reset = "\x1b[0m";
    let up = machine_count - 1 - idx;
    if up > 0 {
        print!("\x1b[{}A", up);
    }
    print!("\r\x1b[2K  {}{}{}{}", color, indicator, reset, text);
    if up > 0 {
        print!("\x1b[{}B", up);
    }
}
