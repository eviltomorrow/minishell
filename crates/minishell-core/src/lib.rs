pub const NOT_EXIST: &str = "-";

#[derive(Debug, Clone)]
pub struct Machine {
    pub id: i64,
    pub num: i32,
    pub nat_ip: String,
    pub ip: String,
    pub username: String,
    pub password: String,
    pub port: i32,
    pub private_key_path: String,
    pub device: String,
    pub remark: String,
}

impl Machine {
    pub fn effective_host(&self) -> &str {
        if !self.nat_ip.is_empty() && self.nat_ip != NOT_EXIST {
            &self.nat_ip
        } else {
            &self.ip
        }
    }

    pub fn is_empty_field(s: &str) -> bool {
        s.is_empty() || s == NOT_EXIST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_machine() -> Machine {
        Machine {
            id: 1,
            num: 0,
            nat_ip: "".into(),
            ip: "192.168.1.1".into(),
            username: "root".into(),
            password: "".into(),
            port: 22,
            private_key_path: "".into(),
            device: "".into(),
            remark: "".into(),
        }
    }

    #[test]
    fn test_effective_host_with_nat() {
        let mut m = test_machine();
        m.nat_ip = "10.0.0.2".into();
        assert_eq!(m.effective_host(), "10.0.0.2");
    }

    #[test]
    fn test_effective_host_without_nat() {
        let m = test_machine();
        assert_eq!(m.effective_host(), "192.168.1.1");
    }

    #[test]
    fn test_effective_host_nat_dash() {
        let mut m = test_machine();
        m.nat_ip = NOT_EXIST.into();
        assert_eq!(m.effective_host(), "192.168.1.1");
    }

    #[test]
    fn test_is_empty_field() {
        assert!(Machine::is_empty_field(""));
        assert!(Machine::is_empty_field(NOT_EXIST));
        assert!(!Machine::is_empty_field("hello"));
    }
}
