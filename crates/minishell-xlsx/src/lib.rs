use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;
use calamine::Reader;

pub fn generate_template(path: &Path) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();

    let headers = ["IP", "NAT-IP", "Port", "Username", "Password", "PrivateKey-Path", "Device", "Remark"];
    for (i, h) in headers.iter().enumerate() {
        sheet.write_string(0, i as u16, *h)?;
    }

    let example = ["10.0.0.1", "-", "22", "root", "your-password", "-", "Linux", "example"];
    for (i, v) in example.iter().enumerate() {
        sheet.write_string(1, i as u16, *v)?;
    }

    workbook.save(path).context("Failed to save template")?;
    Ok(())
}

pub fn import_from(path: &Path) -> Result<Vec<Machine>> {
    let mut workbook = calamine::open_workbook_auto(path).context("Failed to open Excel file")?;
    let sheet_names = workbook.sheet_names();
    let sheet_name = sheet_names.first().context("No sheets found")?;

    let range = workbook.worksheet_range(sheet_name).context("Failed to read sheet")?;
    let rows: Vec<Vec<calamine::Data>> = range.rows().map(|r| r.to_vec()).collect();

    if rows.len() < 2 {
        anyhow::bail!("Excel file must have at least 2 rows (header + data)");
    }

    let mut machines = Vec::new();
    for row in &rows[1..] {
        if row.is_empty() { continue; }

        let ip = cell_to_string(row.get(0));
        if ip.is_empty() || looks_like_description(&ip) { continue; }

        let nat_ip = cell_to_string(row.get(1));
        let port: i32 = cell_to_string(row.get(2)).parse().unwrap_or(22);
        let username = cell_to_string(row.get(3));
        let password = cell_to_string(row.get(4));
        let private_key_path = cell_to_string(row.get(5));
        let device = cell_to_string(row.get(6));
        let remark = cell_to_string(row.get(7));

        machines.push(Machine {
            id: 0,
            num: 0,
            nat_ip: if nat_ip.is_empty() { "-".into() } else { nat_ip },
            ip,
            username: if username.is_empty() { "root".into() } else { username },
            password: if password.is_empty() { "-".into() } else { password },
            port,
            private_key_path: if private_key_path.is_empty() { "-".into() } else { private_key_path },
            device: if device.is_empty() { "-".into() } else { device },
            remark: if remark.is_empty() { "-".into() } else { remark },
        });
    }

    Ok(machines)
}

pub fn export_to(path: &Path, machines: &[Machine]) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();

    let headers = ["#", "IP", "NAT-IP", "Port", "Username", "Password", "PrivateKey", "Device", "Remark"];
    let header_format = rust_xlsxwriter::Format::new()
        .set_bold()
        .set_font_color(rust_xlsxwriter::Color::RGB(0xFFFFFF))
        .set_background_color(rust_xlsxwriter::Color::RGB(0x4472C4))
        .set_align(rust_xlsxwriter::FormatAlign::Center);

    for (i, h) in headers.iter().enumerate() {
        sheet.write_string_with_format(0, i as u16, *h, &header_format)?;
    }

    let stripe_format = rust_xlsxwriter::Format::new()
        .set_background_color(rust_xlsxwriter::Color::RGB(0xD9E2F3));

    for (idx, m) in machines.iter().enumerate() {
        let row = (idx + 1) as u32;

        let values = [
            format!("{}", idx + 1),
            m.ip.clone(),
            m.nat_ip.clone(),
            format!("{}", m.port),
            m.username.clone(),
            m.password.clone(),
            m.private_key_path.clone(),
            m.device.clone(),
            m.remark.clone(),
        ];

        for (col, val) in values.iter().enumerate() {
            if idx % 2 == 1 {
                sheet.write_string_with_format(row, col as u16, val.as_str(), &stripe_format)?;
            } else {
                sheet.write_string(row, col as u16, val.as_str())?;
            }
        }
    }

    // Auto-width columns
    for i in 0..headers.len() {
        sheet.set_column_width(i as u16, 15.0)?;
    }

    sheet.set_freeze_panes(1, 0)?;

    workbook.save(path).context("Failed to save export")?;
    Ok(())
}

fn cell_to_string(cell: Option<&calamine::Data>) -> String {
    match cell {
        Some(calamine::Data::String(s)) => s.trim().to_string(),
        Some(calamine::Data::Float(f)) => {
            if *f == (*f as i64) as f64 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Some(calamine::Data::Int(i)) => format!("{}", i),
        _ => String::new(),
    }
}

fn looks_like_description(s: &str) -> bool {
    if s.is_empty() { return true; }

    // Check for Chinese characters (Unicode Han range)
    if s.chars().any(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp) || (0x3400..=0x4DBF).contains(&cp)
    }) {
        return true;
    }

    // Check for keywords
    let upper = s.to_uppercase();
    if upper.contains("IP") || upper.contains("SSH") || upper.contains("LOCAL") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_description() {
        assert!(looks_like_description(""));
        assert!(looks_like_description("这是一台服务器"));
        assert!(looks_like_description("SSH Connection"));
        assert!(looks_like_description("LOCAL MACHINE"));
        assert!(!looks_like_description("10.0.0.1"));
        assert!(!looks_like_description("web-server"));
    }
}
