//! SQLite persistence: migrations, a single writer thread and read connections.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rusqlite::{Connection, OpenFlags};
use rusqlite_migration::{M, Migrations};
use tokio::sync::{mpsc, oneshot};

static MIGRATIONS: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::new(vec![M::up(include_str!("../migrations/0001_init.sql"))]));

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("store writer thread is gone")]
    WriterGone,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to the database. Cheap to clone; all writes are serialized on one thread.
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    writer: mpsc::UnboundedSender<Job>,
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
}

impl Store {
    /// Open (creating if needed) the database at `path` and apply all migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut conn = Connection::open(&path)?;
        configure(&conn)?;
        MIGRATIONS.to_latest(&mut conn)?;

        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name("termflow-store-writer".into())
            .spawn(move || {
                while let Some(job) = rx.blocking_recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Self { path, writer: tx })
    }

    /// Run `f` on the single writer connection and wait for its result.
    pub async fn write<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.writer
            .send(Box::new(move |c| {
                let _ = tx.send(f(c)); // receiver dropped = caller gave up; nothing to do
            }))
            .map_err(|_| StoreError::WriterGone)?;
        rx.await.map_err(|_| StoreError::WriterGone)?
    }

    /// Open a read-only connection.
    // shortcut: a fresh connection per call, no pool; add a pool when read traffic shows up (M2).
    pub fn reader(&self) -> Result<Connection, StoreError> {
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[tokio::test]
    async fn open_migrates_and_sets_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let store = Store::open(&path).unwrap();
        // reopening is idempotent
        drop(Store::open(&path).unwrap());

        let r = store.reader().unwrap();
        let mode: String = r.query_row("PRAGMA journal_mode", [], |x| x.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = r.query_row("PRAGMA foreign_keys", [], |x| x.get(0)).unwrap();
        assert_eq!(fk, 1);
        for t in ["sessions", "turns", "parts", "attempts", "blocks", "costs", "settings"] {
            let n: i64 = r
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |x| x.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {t}");
        }
    }

    #[tokio::test]
    async fn writes_go_through_writer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        store
            .write(|c| Ok(c.execute("INSERT INTO settings(key, value_json) VALUES ('a', '1')", [])?))
            .await
            .unwrap();
        let v: String = store
            .reader()
            .unwrap()
            .query_row("SELECT value_json FROM settings WHERE key='a'", [], |x| x.get(0))
            .unwrap();
        assert_eq!(v, "1");
    }

    #[tokio::test]
    async fn foreign_keys_enforced_on_writer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let res = store
            .write(|c| {
                Ok(c.execute(
                    "INSERT INTO turns(id, session_id, role, status, created_at, seq) VALUES ('t','missing','user','committed','now',0)",
                    [],
                )?)
            })
            .await;
        assert!(res.is_err());
    }
}
