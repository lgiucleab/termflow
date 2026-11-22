//! Key/value settings (small JSON blobs such as the saved tab layout).

use rusqlite::{Connection, OptionalExtension, params};

use crate::StoreError;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row("SELECT value_json FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set(conn: &Connection, key: &str, value_json: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO settings (key, value_json) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        params![key, value_json],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn upsert() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::open(dir.path().join("t.db")).unwrap();
        store.write(|c| super::set(c, "k", "1")).await.unwrap();
        store.write(|c| super::set(c, "k", "2")).await.unwrap();
        assert_eq!(super::get(&store.reader().unwrap(), "k").unwrap().as_deref(), Some("2"));
        assert_eq!(super::get(&store.reader().unwrap(), "nope").unwrap(), None);
    }
}
