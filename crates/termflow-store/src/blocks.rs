//! Terminal block metadata and output excerpts.

use rusqlite::{Connection, OptionalExtension, Row, params};
use termflow_types::{Block, BlockId};

use crate::StoreError;

fn ts(t: jiff::Timestamp) -> String {
    t.to_string()
}

fn parse_ts(s: &str) -> rusqlite::Result<jiff::Timestamp> {
    s.parse()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

fn parse_id<T: std::str::FromStr>(s: &str) -> rusqlite::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

/// Insert a finished block with its (ANSI-stripped, capped) output.
pub fn insert(conn: &Connection, b: &Block, output: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO blocks (id, terminal_session_id, seq, command, cwd, git_branch, shell, exit_code, started_at, finished_at, output_excerpt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            b.id.to_string(),
            b.terminal_id.to_string(),
            b.seq,
            b.command,
            b.cwd,
            b.git_branch,
            b.shell,
            b.exit_code,
            ts(b.started_at),
            b.finished_at.map(ts),
            output,
        ],
    )?;
    Ok(())
}

const COLS: &str = "id, terminal_session_id, seq, command, cwd, git_branch, shell, exit_code, started_at, finished_at";

fn row(r: &Row<'_>) -> rusqlite::Result<Block> {
    let id: String = r.get(0)?;
    let term: String = r.get(1)?;
    let started: String = r.get(8)?;
    let finished: Option<String> = r.get(9)?;
    Ok(Block {
        id: BlockId(parse_id(&id)?),
        terminal_id: termflow_types::TerminalId(parse_id(&term)?),
        seq: r.get(2)?,
        command: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        cwd: r.get(4)?,
        git_branch: r.get(5)?,
        shell: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        exit_code: r.get(7)?,
        started_at: parse_ts(&started)?,
        finished_at: finished.as_deref().map(parse_ts).transpose()?,
    })
}

pub fn get(conn: &Connection, id: BlockId) -> Result<Option<(Block, String)>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS}, output_excerpt FROM blocks WHERE id = ?1"),
            [id.to_string()],
            |r| Ok((row(r)?, r.get::<_, Option<String>>(10)?.unwrap_or_default())),
        )
        .optional()?)
}

/// Most recently finished blocks first.
pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<Block>, StoreError> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM blocks ORDER BY finished_at DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limit], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use termflow_types::TerminalId;

    #[tokio::test]
    async fn roundtrip_and_recent_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let t0 = jiff::Timestamp::UNIX_EPOCH;
        let mk = |seq: u32, secs: i64| Block {
            id: BlockId::new(),
            terminal_id: TerminalId::new(),
            seq,
            command: format!("cmd{seq}"),
            cwd: Some("/w".into()),
            git_branch: None,
            shell: "zsh".into(),
            exit_code: Some(0),
            started_at: t0,
            finished_at: Some(t0 + jiff::SignedDuration::from_secs(secs)),
        };
        let (a, b) = (mk(0, 1), mk(1, 2));
        let (a2, b2) = (a.clone(), b.clone());
        store
            .write(move |c| {
                insert(c, &a2, "out-a")?;
                insert(c, &b2, "out-b")
            })
            .await
            .unwrap();
        let r = store.reader().unwrap();
        assert_eq!(get(&r, a.id).unwrap().unwrap(), (a.clone(), "out-a".to_owned()));
        assert_eq!(recent(&r, 10).unwrap(), vec![b, a]);
        assert!(get(&r, BlockId::new()).unwrap().is_none());
    }
}
