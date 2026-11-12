use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;
use rusqlite::{Connection, params};

pub struct Store {
    conn: Connection,
}

/// A machine field that `resort` can order by.
///
/// Ordering is applied in SQL so the database does the work; `num` is then
/// renumbered 1..N in that order, which makes the new order the canonical one
/// (`search` sorts by `num`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Ip,
    NatIp,
    Port,
    Username,
    Device,
    Remark,
}

impl SortColumn {
    /// SQL fragment used as the `ORDER BY` clause.
    ///
    /// Empty fields (`''`/`'-'`) always sort last, in both directions, so a
    /// descending sort doesn't drag a wall of placeholders to the top. Ties
    /// fall back to `id` for a deterministic, stable renumbering.
    fn order_clause(self, desc: bool) -> String {
        let dir = if desc { "DESC" } else { "ASC" };
        let name = match self {
            SortColumn::Port => return format!("(port IS NULL) ASC, port {dir}, id ASC"),
            SortColumn::Ip => "ip",
            SortColumn::NatIp => "nat_ip",
            SortColumn::Username => "username",
            SortColumn::Device => "device",
            SortColumn::Remark => "remark",
        };
        format!(
            "({name} IS NULL OR {name} = '' OR {name} = '-') ASC, {name} COLLATE NOCASE {dir}, id ASC"
        )
    }
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
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<Machine> {
            Ok(Machine {
                id: row.get(0)?, num: row.get(1)?, ip: row.get(2)?,
                nat_ip: row.get(3)?, port: row.get(4)?, username: row.get(5)?,
                password: row.get(6)?, private_key_path: row.get(7)?,
                device: row.get(8)?, remark: row.get(9)?,
            })
        };

        if query.is_empty() {
            let mut stmt = self.conn.prepare(
                "SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines ORDER BY num IS NULL, num, id"
            )?;
            let mut mapped = stmt.query_map([], map_row)?;
            let mut rows = Vec::new();
            while let Some(row) = mapped.next() { rows.push(row?); }
            Ok(rows)
        } else {
            let pattern = format!("%{}%", query);
            let mut stmt = self.conn.prepare(
                "SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines WHERE ip LIKE ?1 OR remark LIKE ?1 ORDER BY num IS NULL, num, id"
            )?;
            let mut mapped = stmt.query_map(params![pattern], map_row)?;
            let mut rows = Vec::new();
            while let Some(row) = mapped.next() { rows.push(row?); }
            Ok(rows)
        }
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

    pub fn max_num(&self) -> Result<i32> {
        let max: i32 = self.conn.query_row("SELECT COALESCE(MAX(num), 0) FROM machines", [], |row| row.get(0))?;
        Ok(max)
    }

    /// Reorders every machine by `column` and renumbers `num` 1..N in that
    /// order. Because `search` sorts by `num`, the new order is immediately
    /// visible in the TUI, `show`, and the quick-login selector.
    pub fn resort(&self, column: SortColumn, desc: bool) -> Result<usize> {
        let sql = format!("SELECT id FROM machines ORDER BY {}", column.order_clause(desc));
        let mut stmt = self.conn.prepare(&sql)?;
        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        self.renumber(&ids)
    }

    pub fn reset_num(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare("SELECT id FROM machines ORDER BY id")?;
        let ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        self.renumber(&ids)
    }

    /// Assigns `num` = 1..N to `ids` in order, atomically.
    fn renumber(&self, ids: &[i64]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE machines SET num = ?1 WHERE id = ?2", params![i as i32 + 1, id])?;
        }
        tx.commit()?;
        Ok(ids.len())
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

    #[test]
    fn test_full_crud_flow() {
        let (store, dir) = temp_store();
        
        // Create
        let machines = vec![test_machine("10.0.0.1"), test_machine("10.0.0.2")];
        let inserted = store.import_machines(&machines).unwrap();
        assert_eq!(inserted, 2);
        
        // Read
        let all = store.search("").unwrap();
        assert_eq!(all.len(), 2);
        
        // Update
        let mut m = all[0].clone();
        m.remark = "updated".into();
        store.update_machine(&m).unwrap();
        
        let updated = store.search("10.0.0.1").unwrap();
        assert_eq!(updated[0].remark, "updated");
        
        // Delete
        store.delete_machine(m.id).unwrap();
        assert_eq!(store.search("").unwrap().len(), 1);
        
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_search_by_remark() {
        let (store, dir) = temp_store();
        
        let mut m1 = test_machine("10.0.0.1");
        m1.remark = "web-server".into();
        let mut m2 = test_machine("10.0.0.2");
        m2.remark = "db-server".into();
        
        store.import_machines(&vec![m1, m2]).unwrap();
        
        let results = store.search("web").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].remark, "web-server");
        
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_duplicate_ip_port_skipped() {
        let (store, dir) = temp_store();
        
        let machines = vec![test_machine("10.0.0.1"), test_machine("10.0.0.1")];
        let inserted = store.import_machines(&machines).unwrap();
        assert_eq!(inserted, 1);
        
        let all = store.search("").unwrap();
        assert_eq!(all.len(), 1);
        
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_max_num() {
        let (store, dir) = temp_store();
        
        assert_eq!(store.max_num().unwrap(), 0);
        
        let mut m = test_machine("10.0.0.1");
        m.num = 5;
        store.import_machines(&vec![m]).unwrap();
        
        assert_eq!(store.max_num().unwrap(), 5);
        
        std::fs::remove_dir_all(&dir).ok();
    }

    fn machine_with(ip: &str, nat: &str, port: i32, user: &str, device: &str, remark: &str) -> Machine {
        Machine {
            id: 0,
            num: 0,
            nat_ip: nat.into(),
            ip: ip.into(),
            username: user.into(),
            password: "pass".into(),
            port,
            private_key_path: "-".into(),
            device: device.into(),
            remark: remark.into(),
        }
    }

    /// Asserts that `search("")` returns `expected_ips` in that exact order and
    /// that `num` is sequential 1..N — i.e. the visible order follows `num`.
    fn assert_order(store: &Store, expected_ips: &[&str]) {
        let all = store.search("").unwrap();
        let ips: Vec<&str> = all.iter().map(|m| m.ip.as_str()).collect();
        assert_eq!(ips, expected_ips);
        let nums: Vec<i32> = all.iter().map(|m| m.num).collect();
        assert_eq!(nums, (1..=expected_ips.len() as i32).collect::<Vec<_>>());
    }

    #[test]
    fn resort_by_ip_reorders_and_renumbers() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.3", "-", 22, "root", "Linux", "c"),
            machine_with("10.0.0.1", "-", 22, "root", "Linux", "a"),
            machine_with("10.0.0.2", "-", 22, "root", "Linux", "b"),
        ];
        store.import_machines(&machines).unwrap();

        assert_eq!(store.resort(SortColumn::Ip, false).unwrap(), 3);
        assert_order(&store, &["10.0.0.1", "10.0.0.2", "10.0.0.3"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resort_desc_reverses_order() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.3", "-", 22, "root", "Linux", "c"),
            machine_with("10.0.0.1", "-", 22, "root", "Linux", "a"),
            machine_with("10.0.0.2", "-", 22, "root", "Linux", "b"),
        ];
        store.import_machines(&machines).unwrap();

        assert_eq!(store.resort(SortColumn::Ip, true).unwrap(), 3);
        assert_order(&store, &["10.0.0.3", "10.0.0.2", "10.0.0.1"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resort_puts_empty_values_last_in_both_directions() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.1", "-", 22, "root", "Linux", ""),
            machine_with("10.0.0.2", "-", 22, "root", "Linux", "zeta"),
            machine_with("10.0.0.3", "-", 22, "root", "Linux", "-"),
            machine_with("10.0.0.4", "-", 22, "root", "Linux", "alpha"),
        ];
        store.import_machines(&machines).unwrap();

        store.resort(SortColumn::Remark, false).unwrap();
        assert_order(&store, &["10.0.0.4", "10.0.0.2", "10.0.0.1", "10.0.0.3"]);

        store.resort(SortColumn::Remark, true).unwrap();
        // Non-empty values descend; placeholders stay last ("-" then "" by value).
        assert_order(&store, &["10.0.0.2", "10.0.0.4", "10.0.0.3", "10.0.0.1"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resort_port_is_numeric_not_lexicographic() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.1", "-", 22, "root", "Linux", "x"),
            machine_with("10.0.0.2", "-", 100, "root", "Linux", "x"),
            machine_with("10.0.0.3", "-", 9, "root", "Linux", "x"),
        ];
        store.import_machines(&machines).unwrap();

        store.resort(SortColumn::Port, false).unwrap();
        assert_order(&store, &["10.0.0.3", "10.0.0.1", "10.0.0.2"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resort_username_is_case_insensitive() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.1", "-", 22, "Bob", "Linux", "x"),
            machine_with("10.0.0.2", "-", 22, "alice", "Linux", "x"),
        ];
        store.import_machines(&machines).unwrap();

        store.resort(SortColumn::Username, false).unwrap();
        assert_order(&store, &["10.0.0.2", "10.0.0.1"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resort_ties_fall_back_to_id() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.1", "-", 22, "root", "Linux", "same"),
            machine_with("10.0.0.2", "-", 22, "root", "Linux", "same"),
        ];
        store.import_machines(&machines).unwrap();

        store.resort(SortColumn::Remark, true).unwrap();
        assert_order(&store, &["10.0.0.1", "10.0.0.2"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reset_num_renumbers_by_id_after_resort() {
        let (store, dir) = temp_store();
        let machines = vec![
            machine_with("10.0.0.2", "-", 22, "root", "Linux", "b"),
            machine_with("10.0.0.1", "-", 22, "root", "Linux", "a"),
        ];
        store.import_machines(&machines).unwrap();

        store.resort(SortColumn::Ip, false).unwrap();
        assert_order(&store, &["10.0.0.1", "10.0.0.2"]);

        // resetnum stays id-based, so it restores insertion order.
        assert_eq!(store.reset_num().unwrap(), 2);
        assert_order(&store, &["10.0.0.2", "10.0.0.1"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
