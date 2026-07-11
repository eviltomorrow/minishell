use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;
use calamine::Reader;

const HEADER_BG: rust_xlsxwriter::Color = rust_xlsxwriter::Color::RGB(0x4472C4);
const STRIPE_BG: rust_xlsxwriter::Color = rust_xlsxwriter::Color::RGB(0xD9E2F3);

struct ColDef {
    title: &'static str,
    width: f64,
}

const TEMPLATE_COLS: &[ColDef] = &[
    ColDef { title: "IP", width: 18.0 },
    ColDef { title: "NAT-IP", width: 18.0 },
    ColDef { title: "Port", width: 8.0 },
    ColDef { title: "Username", width: 12.0 },
    ColDef { title: "Password", width: 15.0 },
    ColDef { title: "PrivateKey-Path", width: 22.0 },
    ColDef { title: "Device", width: 12.0 },
    ColDef { title: "Remark", width: 22.0 },
];

const EXPORT_COLS: &[ColDef] = &[
    ColDef { title: "#", width: 5.0 },
    ColDef { title: "IP", width: 18.0 },
    ColDef { title: "NAT-IP", width: 18.0 },
    ColDef { title: "Port", width: 8.0 },
    ColDef { title: "Username", width: 12.0 },
    ColDef { title: "Password", width: 15.0 },
    ColDef { title: "PrivateKey", width: 30.0 },
    ColDef { title: "Device", width: 12.0 },
    ColDef { title: "Remark", width: 20.0 },
];

fn header_format() -> rust_xlsxwriter::Format {
    rust_xlsxwriter::Format::new()
        .set_bold()
        .set_font_color(rust_xlsxwriter::Color::RGB(0xFFFFFF))
        .set_background_color(HEADER_BG)
        .set_align(rust_xlsxwriter::FormatAlign::Center)
        .set_align(rust_xlsxwriter::FormatAlign::VerticalCenter)
        .set_border(rust_xlsxwriter::FormatBorder::Thin)
        .set_border_color(rust_xlsxwriter::Color::RGB(0x8DB4E2))
}

fn cell_format() -> rust_xlsxwriter::Format {
    rust_xlsxwriter::Format::new()
        .set_align(rust_xlsxwriter::FormatAlign::VerticalCenter)
        .set_border(rust_xlsxwriter::FormatBorder::Thin)
        .set_border_color(rust_xlsxwriter::Color::RGB(0xD9D9D9))
}

fn stripe_format() -> rust_xlsxwriter::Format {
    rust_xlsxwriter::Format::new()
        .set_background_color(STRIPE_BG)
        .set_align(rust_xlsxwriter::FormatAlign::VerticalCenter)
        .set_border(rust_xlsxwriter::FormatBorder::Thin)
        .set_border_color(rust_xlsxwriter::Color::RGB(0xD9D9D9))
}

fn write_header(sheet: &mut rust_xlsxwriter::Worksheet, cols: &[ColDef]) -> Result<()> {
    let fmt = header_format();
    for (i, col) in cols.iter().enumerate() {
        sheet.write_string_with_format(0, i as u16, col.title, &fmt)?;
        sheet.set_column_width(i as u16, col.width)?;
    }
    sheet.set_row_height(0, 22.0)?;
    sheet.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_data_row(sheet: &mut rust_xlsxwriter::Worksheet, row: u32, values: &[String], is_stripe: bool) -> Result<()> {
    let fmt = if is_stripe { stripe_format() } else { cell_format() };
    for (col, val) in values.iter().enumerate() {
        sheet.write_string_with_format(row, col as u16, val.as_str(), &fmt)?;
    }
    Ok(())
}

pub fn generate_template(path: &Path) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();

    write_header(sheet, TEMPLATE_COLS)?;

    let example_vals: Vec<String> = ["10.0.0.1", "-", "22", "root", "your-password", "-", "Linux", "example"]
        .iter().map(|s| s.to_string()).collect();
    write_data_row(sheet, 1, &example_vals, false)?;

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

    write_header(sheet, EXPORT_COLS)?;

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

        let vals: Vec<String> = values.iter().map(|v| {
            if v == "-" || v.is_empty() { "-".to_string() } else { v.clone() }
        }).collect();

        write_data_row(sheet, row, &vals, idx % 2 == 1)?;
    }

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
