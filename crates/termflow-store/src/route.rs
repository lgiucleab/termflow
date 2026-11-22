//! Router state: credential blocks, session profiles, spend totals and the catalog cache.

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};
use termflow_types::SessionId;

use crate::StoreError;

pub fn load_blocks(conn: &Connection) -> Result<Vec<(String, Option<Timestamp>, String)>, StoreError> {
    let mut st = conn.prepare("SELECT credential_id, until, reason FROM route_blocks")?;
    let rows = st.query_map([], |r| {
        let until: Option<String> = r.get(1)?;
        Ok((r.get(0)?, until.and_then(|u| u.parse().ok()), r.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn put_block(conn: &Connection, cred: &str, until: Option<Timestamp>, reason: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO route_blocks (credential_id, until, reason) VALUES (?1, ?2, ?3)
         ON CONFLICT(credential_id) DO UPDATE SET until = ?2, reason = ?3",
        params![cred, until.map(|u| u.to_string()), reason],
    )?;
    Ok(())
}

pub fn delete_block(conn: &Connection, cred: &str) -> Result<(), StoreError> {
    conn.execute("DELETE FROM route_blocks WHERE credential_id = ?1", [cred])?;
    Ok(())
}

pub fn set_session_profile(conn: &Connection, id: SessionId, profile: Option<&str>) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET profile = ?2 WHERE id = ?1",
        params![id.to_string(), profile],
    )?;
    Ok(())
}

/// Recorded spend (USD) of attempts started at or after `since`, optionally for one session
/// or one credential. Attempts with unknown prices count as zero.
pub fn spend_since(
    conn: &Connection,
    since: Timestamp,
    session: Option<SessionId>,
    credential: Option<&str>,
) -> Result<f64, StoreError> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(cost_usd), 0) FROM attempts
         WHERE started_at >= ?1 AND (?2 IS NULL OR session_id = ?2) AND (?3 IS NULL OR credential_id = ?3)",
        params![since.to_string(), session.map(|s| s.to_string()), credential],
        |r| r.get(0),
    )?)
}

pub fn catalog_get(conn: &Connection, source: &str) -> Result<Option<(String, Timestamp)>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT body, fetched_at FROM catalog_cache WHERE source = ?1",
            [source],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
        .and_then(|(b, t)| t.parse().ok().map(|t| (b, t))))
}

pub fn catalog_put(conn: &Connection, source: &str, body: &str, at: Timestamp) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO catalog_cache (source, body, fetched_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(source) DO UPDATE SET body = ?2, fetched_at = ?3",
        params![source, body, at.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::Store;
    use crate::agent::{AttemptEnd, AttemptStart, create_session, finish_attempt, get_session, start_attempt};

    #[tokio::test]
    async fn blocks_profiles_spend_and_cache_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("t.db")).unwrap();
        let sid = SessionId::new();
        let until: Timestamp = "2026-06-01T00:00:00Z".parse().unwrap();
        s.write(move |c| {
            create_session(c, sid, None, None, None)?;
            put_block(c, "c1", Some(until), "quota")?;
            put_block(c, "c2", None, "auth")?;
            put_block(c, "c2", None, "auth again")?;
            set_session_profile(c, sid, Some("coding-main"))?;
            for (i, cost) in [Some(0.25), None, Some(0.5)].into_iter().enumerate() {
                let id = termflow_types::AttemptId::new();
                start_attempt(
                    c,
                    &AttemptStart {
                        id,
                        session: sid,
                        turn: termflow_types::TurnId::new(),
                        provider: "anthropic",
                        model: "m",
                        credential_id: Some(if i == 2 { "c2" } else { "c1" }),
                        retry_no: 0,
                    },
                )?;
                finish_attempt(
                    c,
                    id,
                    &AttemptEnd {
                        http_status: Some(200),
                        error: None,
                        ttft_ms: None,
                        usage: termflow_types::Usage::default(),
                        served_model: None,
                        cost_usd: cost,
                    },
                )?;
            }
            catalog_put(c, "models.dev", "{}", until)?;
            Ok(())
        })
        .await
        .unwrap();
        let c = s.reader().unwrap();
        let mut blocks = load_blocks(&c).unwrap();
        blocks.sort();
        assert_eq!(blocks[0], ("c1".into(), Some(until), "quota".into()));
        assert_eq!(blocks[1], ("c2".into(), None, "auth again".into()));
        assert_eq!(
            get_session(&c, sid).unwrap().unwrap().profile.as_deref(),
            Some("coding-main")
        );
        let early: Timestamp = "2000-01-01T00:00:00Z".parse().unwrap();
        assert!((spend_since(&c, early, Some(sid), None).unwrap() - 0.75).abs() < 1e-9);
        assert!((spend_since(&c, early, None, Some("c1")).unwrap() - 0.25).abs() < 1e-9);
        let future: Timestamp = "2999-01-01T00:00:00Z".parse().unwrap();
        assert!(spend_since(&c, future, None, None).unwrap().abs() < 1e-9);
        assert_eq!(catalog_get(&c, "models.dev").unwrap(), Some(("{}".into(), until)));
        drop(c);
        s.write(|c| delete_block(c, "c1")).await.unwrap();
        assert_eq!(load_blocks(&s.reader().unwrap()).unwrap().len(), 1);
    }
}
