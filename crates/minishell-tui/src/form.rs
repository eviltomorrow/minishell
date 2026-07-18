use minishell_core::Machine;

pub const FORM_FIELDS: &[(&str, usize, usize)] = &[
    ("IP:", 64, 40),
    ("NAT-IP:", 64, 40),
    ("Port:", 5, 10),
    ("Username:", 64, 40),
    ("Password:", 64, 40),
    ("PrivateKey:", 64, 40),
    ("Device:", 64, 40),
    ("Remark:", 64, 40),
];

pub struct FormField {
    pub label: String,
    pub value: String,
    pub max_length: usize,
    pub width: usize,
    pub cursor_pos: usize,
    pub select_options: Option<Vec<String>>,
    pub select_index: usize,
}

impl FormField {
    pub fn new(label: &str, max_length: usize, width: usize) -> Self {
        FormField {
            label: label.to_string(),
            value: String::new(),
            max_length,
            width,
            cursor_pos: 0,
            select_options: None,
            select_index: 0,
        }
    }

    pub fn new_select(label: &str, options: Vec<String>) -> Self {
        FormField {
            label: label.to_string(),
            value: options[0].clone(),
            max_length: 0,
            width: 0,
            cursor_pos: 0,
            select_options: Some(options),
            select_index: 0,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.value.len() + c.len_utf8() <= self.max_length {
            self.value.insert(self.cursor_pos, c);
            self.cursor_pos += c.len_utf8();
        }
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.value[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_pos = prev;
            self.value.remove(prev);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.value[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor_pos = prev;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.value.len() {
            let next = self.value[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.value.len());
            self.cursor_pos = next;
        }
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            if self.value.len() + c.len_utf8() <= self.max_length {
                self.value.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
        }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor_pos = 0;
    }
}

pub struct FormState {
    pub fields: Vec<FormField>,
    pub step: usize,
    pub is_edit: bool,
    pub target_id: Option<i64>,
    pub num: i32,
    pub error: Option<String>,
}

impl FormState {
    pub fn new_add() -> Self {
        let mut fields: Vec<FormField> = FORM_FIELDS.iter()
            .map(|(label, max_len, width)| FormField::new(label, *max_len, *width))
            .collect();
        fields[6] = FormField::new_select("Device:", vec!["Linux".into(), "Router".into(), "Switch".into(), "Other".into()]);
        FormState { fields, step: 0, is_edit: false, target_id: None, num: 0, error: None }
    }

    pub fn new_edit(machine: &Machine) -> Self {
        let values = vec![
            machine.ip.clone(),
            machine.nat_ip.clone(),
            machine.port.to_string(),
            machine.username.clone(),
            machine.password.clone(),
            machine.private_key_path.clone(),
            machine.device.clone(),
            machine.remark.clone(),
        ];

        let fields: Vec<FormField> = FORM_FIELDS.iter().enumerate()
            .map(|(i, (label, max_len, width))| {
                if i == 6 {
                    let options = vec!["Linux".into(), "Router".into(), "Switch".into(), "Other".into()];
                    let val = &values[i];
                    let idx = if val == "-" || val.is_empty() { 0 } else { options.iter().position(|o| o == val).unwrap_or(0) };
                    let mut f = FormField::new_select(label, options);
                    f.select_index = idx;
                    f.value = f.select_options.as_ref().unwrap()[idx].clone();
                    f
                } else {
                    let mut f = FormField::new(label, *max_len, *width);
                    let val = &values[i];
                    f.value = if val.is_empty() { "-".to_string() } else { val.clone() };
                    f.cursor_pos = f.value.len();
                    f
                }
            })
            .collect();

        FormState { fields, step: 0, is_edit: true, target_id: Some(machine.id), num: machine.num, error: None }
    }

    pub fn navigate_next(&mut self) {
        self.step = (self.step + 1) % self.fields.len();
    }

    pub fn navigate_prev(&mut self) {
        self.step = if self.step == 0 { self.fields.len() - 1 } else { self.step - 1 };
    }

    pub fn validate(&self) -> Option<&str> {
        for (i, field) in self.fields.iter().enumerate() {
            if field.select_options.is_some() {
                continue;
            }
            if i == 4 || i == 5 {
                continue;
            }
            if field.value.contains(' ') {
                return Some("字段不能包含空格");
            }
        }

        let ip = self.fields[0].value.trim();
        let nat_ip = self.fields[1].value.trim();
        let port = self.fields[2].value.trim();
        let username = self.fields[3].value.trim();
        let password = self.fields[4].value.trim();
        let private_key = self.fields[5].value.trim();

        let empty = |s: &str| s.is_empty() || s == "-";

        if empty(ip) && empty(nat_ip) {
            return Some("IP 和 NAT-IP 不能同时为空");
        }
        if empty(username) {
            return Some("用户名不能为空");
        }
        if port.is_empty() {
            return Some("端口不能为空");
        }
        if port.parse::<u16>().is_err() {
            return Some("端口格式无效");
        }
        if empty(password) && empty(private_key) {
            return Some("密码和密钥不能同时为空");
        }

        None
    }

    pub fn to_machine(&self) -> Machine {
        let port: i32 = self.fields[2].value.parse().unwrap_or(22);
        let or_dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };

        Machine {
            id: self.target_id.unwrap_or(0),
            num: self.num,
            ip: or_dash(&self.fields[0].value),
            nat_ip: or_dash(&self.fields[1].value),
            port,
            username: or_dash(&self.fields[3].value),
            password: or_dash(&self.fields[4].value),
            private_key_path: or_dash(&self.fields[5].value),
            device: or_dash(&self.fields[6].value),
            remark: or_dash(&self.fields[7].value),
        }
    }
}

pub struct DeleteState {
    pub target: Machine,
}

impl DeleteState {
    pub fn new(target: Machine) -> Self {
        DeleteState { target }
    }
}
