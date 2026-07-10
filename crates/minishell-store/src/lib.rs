use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;
use rusqlite::{Connection, params};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path).context("Failed to create store directory")?;
        let db_path = path.join("db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL")?;
        Ok(Store { conn })
    }

    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS machines (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                num         INTEGER,
                ip          TEXT NOT NULL,
                nat_ip      TEXT DEFAULT '',
                port        INTEGER DEFAULT 22,
                username    TEXT NOT NULL,
                password    TEXT DEFAULT '',
                private_key TEXT DEFAULT '',
                device      TEXT DEFAULT '',
                remark      TEXT DEFAULT '',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_machines_ip_port ON machines(ip, port);"
        ).context("Failed to initialize database schema")?;
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<Machine>> {
        let sql = if query.is_empty() {
            "SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines ORDER BY id"
        } else {
            "SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines WHERE ip LIKE ?1 OR remark LIKE ?1 ORDER BY id"
        };

        let mut stmt = self.conn.prepare(sql)?;

        let rows = if query.is_empty() {
            let mut rows = Vec::new();
            let mut mapped = stmt.query_map([], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    num: row.get(1)?,
                    ip: row.get(2)?,
                    nat_ip: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    password: row.get(6)?,
                    private_key_path: row.get(7)?,
                    device: row.get(8)?,
                    remark: row.get(9)?,
                })
            })?;
            while let Some(row) = mapped.next() {
                rows.push(row?);
            }
            rows
        } else {
            let pattern = format!("%{}%", query);
            let mut rows = Vec::new();
            let mut mapped = stmt.query_map(params![pattern], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    num: row.get(1)?,
                    ip: row.get(2)?,
                    nat_ip: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    password: row.get(6)?,
                    private_key_path: row.get(7)?,
                    device: row.get(8)?,
                    remark: row.get(9)?,
                })
            })?;
            while let Some(row) = mapped.next() {
                rows.push(row?);
            }
            rows
        };

        Ok(rows)
    }

    pub fn count_all(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM machines", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn import_machines(&self, machines: &[Machine]) -> Result<usize> {
        let mut inserted = 0;
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO machines (num, ip, nat_ip, port, username, password, private_key, device, remark)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            )?;

            for m in machines {
                let changes = stmt.execute(params![
                    m.num, m.ip, m.nat_ip, m.port, m.username, m.password,
                    m.private_key_path, m.device, m.remark
                ])?;
                inserted += changes;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn update_machine(&self, m: &Machine) -> Result<()> {
        self.conn.execute(
            "UPDATE machines SET num=?1, ip=?2, nat_ip=?3, port=?4, username=?5, password=?6, private_key=?7, device=?8, remark=?9, updated_at=datetime('now') WHERE id=?10",
            params![m.num, m.ip, m.nat_ip, m.port, m.username, m.password, m.private_key_path, m.device, m.remark, m.id],
        )?;
        Ok(())
    }

    pub fn delete_machine(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM machines WHERE id=?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_store() -> (Store, PathBuf) {
        let dir = PathBuf::from(format!("/tmp/minishell_test_{}_{:?}", std::process::id(), std::time::Instant::now()));
        let store = Store::open(&dir).unwrap();
        store.init().unwrap();
        (store, dir)
    }

    fn test_machine(ip: &str) -> Machine {
        Machine {
            id: 0,
            num: 0,
            nat_ip: "".into(),
            ip: ip.into(),
            username: "root".into(),
            password: "pass".into(),
            port: 22,
            private_key_path: "".into(),
            device: "Linux".into(),
            remark: "test".into(),
        }
    }

    #[test]
    fn test_import_and_search() {
        let (store, dir) = temp_store();
        let machines = vec![test_machine("10.0.0.1"), test_machine("10.0.0.2")];
        let inserted = store.import_machines(&machines).unwrap();
        assert_eq!(inserted, 2);

        let all = store.search("").unwrap();
        assert_eq!(all.len(), 2);

        let found = store.search("10.0.0.1").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ip, "10.0.0.1");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_update_and_delete() {
        let (store, dir) = temp_store();
        store.import_machines(&vec![test_machine("10.0.0.1")]).unwrap();
        let mut m = store.search("10.0.0.1").unwrap().remove(0);
        m.remark = "updated".into();
        store.update_machine(&m).unwrap();
        let updated = store.search("10.0.0.1").unwrap();
        assert_eq!(updated[0].remark, "updated");

        store.delete_machine(m.id).unwrap();
        assert_eq!(store.search("").unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_count_all() {
        let (store, dir) = temp_store();
        assert_eq!(store.count_all().unwrap(), 0);
        store.import_machines(&vec![test_machine("10.0.0.1")]).unwrap();
        assert_eq!(store.count_all().unwrap(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
