//! Agent sessions, the append-only transcript, credentials metadata and the attempt log.

use rusqlite::{Connection, OptionalExtension, Row, params};
use termflow_types::{
    AgentSession, AttemptId, Credential, ErrorClass, Part, ProviderKind, Role, ServedBy, SessionId, Turn, TurnId,
    TurnStatus, Usage, WireFormat,
};

use crate::StoreError;

fn enum_str<T: serde::Serialize>(v: &T) -> Result<String, StoreError> {
    Ok(serde_json::to_value(v)?.as_str().unwrap_or_default().to_owned())
}

fn enum_parse<T: serde::de::DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

fn parse<T: std::str::FromStr>(s: &str) -> rusqlite::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

// ---- sessions ----

pub fn create_session(
    conn: &Connection,
    id: SessionId,
    credential_id: Option<&str>,
    model: Option<&str>,
) -> Result<(), StoreError> {
    let now = jiff::Timestamp::now().to_string();
    conn.execute(
        "INSERT INTO sessions (id, kind, status, credential_id, pinned_model, mode, created_at, updated_at)
         VALUES (?1, 'agent', 'active', ?2, ?3, 'build', ?4, ?4)",
        params![id.to_string(), credential_id, model, now],
    )?;
    Ok(())
}

pub fn set_session_model(
    conn: &Connection,
    id: SessionId,
    credential_id: &str,
    model: &str,
    reasoning: bool,
) -> Result<(), StoreError> {
    let n = conn.execute(
        "UPDATE sessions SET credential_id = ?2, pinned_model = ?3, reasoning = ?4, updated_at = ?5 WHERE id = ?1",
        params![
            id.to_string(),
            credential_id,
            model,
            reasoning,
            jiff::Timestamp::now().to_string()
        ],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(id.to_string()));
    }
    Ok(())
}

pub fn set_session_title(conn: &Connection, id: SessionId, title: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET title = ?2 WHERE id = ?1",
        params![id.to_string(), title],
    )?;
    Ok(())
}

fn session_row(r: &Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        id: SessionId(parse(&r.get::<_, String>(0)?)?),
        title: r.get(1)?,
        credential_id: r.get(2)?,
        model: r.get(3)?,
        reasoning: r.get(4)?,
        updated_at: parse(&r.get::<_, String>(5)?)?,
    })
}

const SESSION_COLS: &str = "id, title, credential_id, pinned_model, reasoning, updated_at";

pub fn get_session(conn: &Connection, id: SessionId) -> Result<Option<AgentSession>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
            [id.to_string()],
            session_row,
        )
        .optional()?)
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<AgentSession>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM sessions WHERE kind = 'agent' ORDER BY updated_at DESC"
    ))?;
    Ok(stmt.query_map([], session_row)?.collect::<Result<_, _>>()?)
}

// ---- transcript ----

/// Append a turn (with its parts) at the end of its session.
pub fn append_turn(conn: &mut Connection, t: &Turn) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    let seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), -1) + 1 FROM turns WHERE session_id = ?1",
        [t.session_id.to_string()],
        |r| r.get(0),
    )?;
    let sb = t.served_by.as_ref();
    tx.execute(
        "INSERT INTO turns (id, session_id, role, status, served_provider, served_model, credential_id, attempt_id, created_at, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            t.id.to_string(),
            t.session_id.to_string(),
            enum_str(&t.role)?,
            enum_str(&t.status)?,
            sb.map(|s| s.provider.clone()),
            sb.map(|s| s.model.clone()),
            sb.and_then(|s| s.credential_id.clone()),
            sb.map(|s| s.attempt_id.to_string()),
            t.created_at.to_string(),
            seq,
        ],
    )?;
    insert_parts(&tx, t.id, &t.parts)?;
    tx.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
        params![t.session_id.to_string(), jiff::Timestamp::now().to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

fn insert_parts(conn: &Connection, turn: TurnId, parts: &[Part]) -> Result<(), StoreError> {
    for (i, p) in (0_i64..).zip(parts) {
        let json = serde_json::to_value(p)?;
        let kind = json.get("type").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
        conn.execute(
            "INSERT INTO parts (id, turn_id, idx, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![format!("{turn}:{i}"), turn.to_string(), i, kind, json.to_string()],
        )?;
    }
    Ok(())
}

/// Finish a streamed turn: write its parts and mark it committed, atomically.
pub fn commit_turn(conn: &mut Connection, t: &Turn) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM parts WHERE turn_id = ?1", [t.id.to_string()])?;
    insert_parts(&tx, t.id, &t.parts)?;
    let sb = t.served_by.as_ref();
    tx.execute(
        "UPDATE turns SET status = 'committed', served_provider = ?2, served_model = ?3, credential_id = ?4, attempt_id = ?5 WHERE id = ?1",
        params![
            t.id.to_string(),
            sb.map(|s| s.provider.clone()),
            sb.map(|s| s.model.clone()),
            sb.and_then(|s| s.credential_id.clone()),
            sb.map(|s| s.attempt_id.to_string()),
        ],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn set_turn_status(conn: &Connection, id: TurnId, status: TurnStatus) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE turns SET status = ?2 WHERE id = ?1",
        params![id.to_string(), enum_str(&status)?],
    )?;
    Ok(())
}

/// Startup recovery: a turn still `partial` means the app died mid-stream. Never resend it.
pub fn discard_partial_turns(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.execute("UPDATE turns SET status = 'discarded' WHERE status = 'partial'", [])?)
}

/// The conversation as models see it: committed turns, in order.
pub fn committed_turns(conn: &Connection, session: SessionId) -> Result<Vec<Turn>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, role, status, served_provider, served_model, credential_id, attempt_id, created_at
         FROM turns WHERE session_id = ?1 AND status = 'committed' ORDER BY seq",
    )?;
    let rows: Vec<(String, Turn)> = stmt
        .query_map([session.to_string()], |r| {
            let id: String = r.get(0)?;
            let provider: Option<String> = r.get(3)?;
            let attempt: Option<String> = r.get(6)?;
            let served_by = match (provider, attempt) {
                (Some(provider), Some(a)) => Some(ServedBy {
                    provider,
                    model: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    credential_id: r.get(5)?,
                    attempt_id: AttemptId(parse(&a)?),
                }),
                _ => None,
            };
            Ok((
                id.clone(),
                Turn {
                    id: TurnId(parse(&id)?),
                    session_id: session,
                    role: enum_parse::<Role>(&r.get::<_, String>(1)?)?,
                    parts: vec![],
                    created_at: parse(&r.get::<_, String>(7)?)?,
                    status: enum_parse(&r.get::<_, String>(2)?)?,
                    served_by,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut parts_stmt = conn.prepare("SELECT json FROM parts WHERE turn_id = ?1 ORDER BY idx")?;
    rows.into_iter()
        .map(|(id, mut t)| {
            let jsons: Vec<String> = parts_stmt.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
            t.parts = jsons
                .iter()
                .map(|j| serde_json::from_str(j))
                .collect::<Result<_, _>>()?;
            Ok(t)
        })
        .collect()
}

// ---- credentials (metadata only) ----

pub fn insert_credential(conn: &Connection, c: &Credential) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO credentials (id, kind, label, base_url, wire, has_secret, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![c.id, enum_str(&c.kind)?, c.label, c.base_url, enum_str(&c.wire)?, c.has_secret, c.created_at.to_string()],
    )?;
    Ok(())
}

fn credential_row(r: &Row<'_>) -> rusqlite::Result<Credential> {
    Ok(Credential {
        id: r.get(0)?,
        kind: enum_parse::<ProviderKind>(&r.get::<_, String>(1)?)?,
        label: r.get(2)?,
        base_url: r.get(3)?,
        wire: enum_parse::<WireFormat>(&r.get::<_, String>(4)?)?,
        has_secret: r.get(5)?,
        created_at: parse(&r.get::<_, String>(6)?)?,
    })
}

pub fn list_credentials(conn: &Connection) -> Result<Vec<Credential>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, label, base_url, wire, has_secret, created_at FROM credentials ORDER BY created_at",
    )?;
    Ok(stmt.query_map([], credential_row)?.collect::<Result<_, _>>()?)
}

pub fn get_credential(conn: &Connection, id: &str) -> Result<Option<Credential>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id, kind, label, base_url, wire, has_secret, created_at FROM credentials WHERE id = ?1",
            [id],
            credential_row,
        )
        .optional()?)
}

pub fn delete_credential(conn: &Connection, id: &str) -> Result<(), StoreError> {
    conn.execute("DELETE FROM credentials WHERE id = ?1", [id])?;
    Ok(())
}

// ---- attempt log (`BUILD_PROMPT` §7.7) ----

pub struct AttemptStart<'a> {
    pub id: AttemptId,
    pub session: SessionId,
    pub turn: TurnId,
    pub provider: &'a str,
    pub model: &'a str,
    pub credential_id: Option<&'a str>,
    pub retry_no: u32,
}

pub fn start_attempt(conn: &Connection, a: &AttemptStart<'_>) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO attempts (id, session_id, turn_id, provider, model, credential_id, started_at, retry_no)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            a.id.to_string(),
            a.session.to_string(),
            a.turn.to_string(),
            a.provider,
            a.model,
            a.credential_id,
            jiff::Timestamp::now().to_string(),
            a.retry_no,
        ],
    )?;
    Ok(())
}

pub struct AttemptEnd<'a> {
    pub http_status: Option<u16>,
    pub error: Option<&'a ErrorClass>,
    pub ttft_ms: Option<u64>,
    pub usage: Usage,
    pub served_model: Option<&'a str>,
}

pub fn finish_attempt(conn: &Connection, id: AttemptId, e: &AttemptEnd<'_>) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE attempts SET ended_at = ?2, http_status = ?3, error_class = ?4, ttft_ms = ?5, input_tokens = ?6,
         output_tokens = ?7, cache_write_tokens = ?8, cache_read_tokens = ?9, reasoning_tokens = ?10,
         model = COALESCE(?11, model) WHERE id = ?1",
        params![
            id.to_string(),
            jiff::Timestamp::now().to_string(),
            e.http_status,
            e.error.map(ErrorClass::name),
            e.ttft_ms.and_then(|v| i64::try_from(v).ok()),
            e.usage.input,
            e.usage.output,
            e.usage.cache_write,
            e.usage.cache_read,
            e.usage.reasoning,
            e.served_model,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("t.db")).unwrap();
        (d, s)
    }

    #[tokio::test]
    async fn transcript_roundtrip_and_crash_restore() {
        let (_d, store) = store();
        let sid = SessionId::new();
        let user = Turn::new(
            sid,
            Role::User,
            vec![Part::Text { text: "hi".into() }],
            TurnStatus::Committed,
        );
        let mut asst = Turn::new(sid, Role::Assistant, vec![], TurnStatus::Partial);
        let (u2, a2) = (user.clone(), asst.clone());
        store
            .write(move |c| {
                create_session(c, sid, None, None)?;
                append_turn(c, &u2)?;
                append_turn(c, &a2)
            })
            .await
            .unwrap();
        // Partial turns are invisible to the model.
        assert_eq!(
            committed_turns(&store.reader().unwrap(), sid).unwrap(),
            vec![user.clone()]
        );

        // Commit with reasoning + served_by.
        asst.parts = vec![
            Part::Reasoning {
                family: "anthropic".into(),
                model: "m".into(),
                credential_scope: "c".into(),
                native: serde_json::json!({"type":"thinking","thinking":"t","signature":"s"}),
                summary_text: Some("t".into()),
            },
            Part::Text { text: "hello".into() },
        ];
        asst.status = TurnStatus::Committed;
        asst.served_by = Some(ServedBy {
            provider: "anthropic".into(),
            model: "m".into(),
            credential_id: None,
            attempt_id: AttemptId::new(),
        });
        let a3 = asst.clone();
        store.write(move |c| commit_turn(c, &a3)).await.unwrap();
        assert_eq!(
            committed_turns(&store.reader().unwrap(), sid).unwrap(),
            vec![user.clone(), asst]
        );

        // A turn left partial (crash) is discarded on startup and never resent.
        let crashed = Turn::new(sid, Role::Assistant, vec![], TurnStatus::Partial);
        store.write(move |c| append_turn(c, &crashed)).await.unwrap();
        assert_eq!(store.write(|c| discard_partial_turns(c)).await.unwrap(), 1);
        assert_eq!(committed_turns(&store.reader().unwrap(), sid).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn credentials_hold_no_secret() {
        let (_d, store) = store();
        let c = Credential {
            id: "c1".into(),
            kind: ProviderKind::Anthropic,
            label: "personal".into(),
            base_url: "https://api.anthropic.com".into(),
            wire: WireFormat::AnthropicMessages,
            has_secret: true,
            created_at: jiff::Timestamp::now(),
        };
        let c2 = c.clone();
        store.write(move |conn| insert_credential(conn, &c2)).await.unwrap();
        let r = store.reader().unwrap();
        assert_eq!(list_credentials(&r).unwrap(), vec![c.clone()]);
        assert_eq!(get_credential(&r, "c1").unwrap(), Some(c));
        let cols: Vec<String> = r
            .prepare("SELECT name FROM pragma_table_info('credentials')")
            .unwrap()
            .query_map([], |x| x.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            !cols.iter().any(|c| c.contains("secret") && c != "has_secret"),
            "{cols:?}"
        );
    }
}
