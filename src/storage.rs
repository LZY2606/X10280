//! SQLite persistence with append-only raw inputs and event sourcing.
//!
//! Immutability is enforced at the database level: triggers reject UPDATE and
//! DELETE on `observations` and `sources`. Replay is possible because every
//! derived row records its algorithm version, parameter fingerprint and the
//! direct source ids it was computed from.

use crate::error::{DomainError, DomainResult};
use crate::model::{Event, Observation, Source};
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub const ALGO_VERSION: &str = "frame-ribbon-1.0.0";

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL,
    case_pack TEXT
);

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    seq INTEGER NOT NULL,
    at_ns INTEGER NOT NULL,
    kind TEXT NOT NULL,
    parent_event_id TEXT,
    payload TEXT NOT NULL,
    sources TEXT NOT NULL,
    algo_version TEXT,
    param_fingerprint TEXT,
    result_id TEXT,
    superseded_by TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS events_order ON events(project_id, seq);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    rate_raw TEXT NOT NULL,
    rate_num INTEGER NOT NULL,
    rate_den INTEGER NOT NULL,
    df INTEGER NOT NULL,
    color_frame INTEGER NOT NULL,
    created_event TEXT NOT NULL,
    PRIMARY KEY (project_id, id)
);

CREATE TABLE IF NOT EXISTS observations (
    id TEXT NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id),
    source_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    raw_line TEXT NOT NULL,
    raw_bits TEXT,
    raw_rate TEXT NOT NULL,
    raw_tc TEXT,
    raw_device_clock TEXT,
    device_clock_ns INTEGER,
    fingerprint TEXT,
    field_mode TEXT NOT NULL,
    diagnostics TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, source_id, seq)
);

CREATE TABLE IF NOT EXISTS params (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    seq INTEGER NOT NULL,
    parent_id TEXT,
    payload TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_event TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS results (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    kind TEXT NOT NULL,
    algo_version TEXT NOT NULL,
    param_fingerprint TEXT NOT NULL,
    sources TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_event TEXT NOT NULL,
    created_seq INTEGER NOT NULL,
    superseded_event TEXT
);
CREATE INDEX IF NOT EXISTS results_project ON results(project_id, kind);

CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    target_result_id TEXT NOT NULL,
    resolution TEXT NOT NULL,
    rationale TEXT NOT NULL,
    event_id TEXT NOT NULL,
    seq INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS archives (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    at_ns INTEGER NOT NULL,
    payload TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS observations_no_update
BEFORE UPDATE ON observations
BEGIN
    SELECT RAISE(ABORT, 'observations are immutable; submit a new version instead');
END;
CREATE TRIGGER IF NOT EXISTS observations_no_delete
BEFORE DELETE ON observations
BEGIN
    SELECT RAISE(ABORT, 'observations are immutable and cannot be deleted');
END;
CREATE TRIGGER IF NOT EXISTS sources_no_update
BEFORE UPDATE ON sources
BEGIN
    SELECT RAISE(ABORT, 'sources are immutable; submit a new version instead');
END;
CREATE TRIGGER IF NOT EXISTS sources_no_delete
BEFORE DELETE ON sources
BEGIN
    SELECT RAISE(ABORT, 'sources cannot be deleted');
END;
"#;

impl Store {
    pub fn open(path: &str) -> DomainResult<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn in_memory() -> DomainResult<Store> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

// ---- low level helpers -----------------------------------------------------

pub fn create_project(conn: &Connection, id: &str, title: &str, at_ns: i64, case_pack: Option<&str>) -> DomainResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO projects(id,title,created_at_ns,case_pack) VALUES(?1,?2,?3,?4)",
        params![id, title, at_ns, case_pack],
    )?;
    Ok(())
}

pub fn project_exists(conn: &Connection, id: &str) -> DomainResult<bool> {
    let mut s = conn.prepare("SELECT 1 FROM projects WHERE id=?1")?;
    Ok(s.exists(params![id])?)
}

pub fn next_seq(conn: &Connection, project_id: &str) -> DomainResult<i64> {
    let v: Option<i64> = conn
        .prepare("SELECT MAX(seq) FROM events WHERE project_id=?1")?
        .query_row(params![project_id], |r| r.get(0))?;
    Ok(v.unwrap_or(0) + 1)
}

pub fn append_event(conn: &Connection, project_id: &str, ev: &Event, at_ns: i64) -> DomainResult<()> {
    conn.execute(
        "INSERT INTO events(id,project_id,seq,at_ns,kind,parent_event_id,payload,sources,
            algo_version,param_fingerprint,result_id,superseded_by)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            ev.id,
            project_id,
            ev.seq,
            at_ns,
            ev.kind,
            ev.parent_event_id,
            serde_json::to_string(&ev.payload)?,
            serde_json::to_string(&ev.sources)?,
            ev.algo_version,
            ev.param_fingerprint,
            ev.result_id,
            ev.superseded_by,
        ],
    )?;
    Ok(())
}

pub fn insert_source(conn: &Connection, project_id: &str, s: &Source) -> DomainResult<()> {
    conn.execute(
        "INSERT INTO sources(id,project_id,name,kind,rate_raw,rate_num,rate_den,df,color_frame,created_event)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            s.id, project_id, s.name,
            format!("{:?}", s.kind).to_ascii_lowercase(),
            s.rate_raw, s.rate.num, s.rate.den, s.df as i64, s.color_frame as i64, s.created_event
        ],
    )?;
    Ok(())
}

pub fn insert_observation(conn: &Connection, project_id: &str, o: &Observation) -> DomainResult<String> {
    let id = crate::hashing::Hasher::id(&["obs", project_id, &o.source_id, &o.seq.to_string(), &o.raw_line]);
    conn.execute(
        "INSERT INTO observations(id,project_id,source_id,seq,kind,raw_line,raw_bits,raw_rate,
            raw_tc,raw_device_clock,device_clock_ns,fingerprint,field_mode,diagnostics)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id, project_id, o.source_id, o.seq,
            format!("{:?}", o.kind).to_ascii_lowercase(),
            o.raw_line, o.raw_bits, o.raw_rate, o.raw_tc, o.raw_device_clock,
            o.device_clock_ns, o.fingerprint,
            format!("{:?}", o.field_mode).to_ascii_lowercase(),
            serde_json::to_string(&o.diagnostics)?,
        ],
    )?;
    Ok(id)
}

pub fn list_events(conn: &Connection, project_id: &str) -> DomainResult<Vec<Event>> {
    let mut s = conn.prepare(
        "SELECT id,seq,at_ns,kind,parent_event_id,payload,sources,algo_version,
                param_fingerprint,result_id,superseded_by
         FROM events WHERE project_id=?1 ORDER BY seq",
    )?;
    let rows = s.query_map(params![project_id], row_to_event)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let payload: String = r.get(5)?;
    let sources: String = r.get(6)?;
    Ok(Event {
        id: r.get(0)?,
        seq: r.get(1)?,
        at_ns: r.get(2)?,
        kind: r.get(3)?,
        parent_event_id: r.get(4)?,
        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
        sources: serde_json::from_str(&sources).unwrap_or_default(),
        algo_version: r.get(7)?,
        param_fingerprint: r.get(8)?,
        result_id: r.get(9)?,
        superseded_by: r.get(10)?,
    })
}

pub fn list_sources(conn: &Connection, project_id: &str) -> DomainResult<Vec<Source>> {
    use crate::rat::Rat;
    let mut s = conn.prepare(
        "SELECT id,name,kind,rate_raw,rate_num,rate_den,df,color_frame,created_event
         FROM sources WHERE project_id=?1 ORDER BY rowid",
    )?;
    let rows = s.query_map(params![project_id], |r| {
        let kind: String = r.get(2)?;
        Ok(Source {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: serde_json::from_str(&format!("\"{kind}\"")).unwrap(),
            rate_raw: r.get(3)?,
            rate: Rat::new(r.get(4)?, r.get(5)?),
            df: r.get::<_, i64>(6)? != 0,
            color_frame: r.get::<_, i64>(7)? != 0,
            created_event: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn list_observations(conn: &Connection, project_id: &str) -> DomainResult<Vec<Observation>> {
    use crate::rat::Rat;
    let mut s = conn.prepare(
        "SELECT o.source_id,o.seq,o.kind,o.raw_line,o.raw_bits,o.raw_rate,o.raw_tc,o.raw_device_clock,
                o.device_clock_ns,o.fingerprint,o.field_mode,o.diagnostics,src.df
         FROM observations o JOIN sources src ON src.project_id=o.project_id AND src.id=o.source_id
         WHERE o.project_id=?1 ORDER BY o.seq",
    )?;
    let rows = s.query_map(params![project_id], |r| {
        let kind: String = r.get(2)?;
        let fm: String = r.get(10)?;
        let diag: String = r.get(11)?;
        let src_df: i64 = r.get(12)?;
        let rate_raw: String = r.get(5)?;
        Ok(Observation {
            source_id: r.get(0)?,
            seq: r.get(1)?,
            kind: serde_json::from_str(&format!("\"{kind}\"")).unwrap(),
            raw_line: r.get(3)?,
            raw_bits: r.get(4)?,
            raw_rate: rate_raw.clone(),
            raw_tc: r.get(6)?,
            raw_device_clock: r.get(7)?,
            device_clock_ns: r.get(8)?,
            fingerprint: r.get(9)?,
            rate: Rat::parse(&rate_raw).unwrap(),
            df: src_df != 0,
            field_mode: serde_json::from_str(&format!("\"{fm}\"")).unwrap(),
            diagnostics: serde_json::from_str(&diag).unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn list_projects(conn: &Connection) -> DomainResult<Vec<(String, String)>> {
    let mut s = conn.prepare("SELECT id,title FROM projects ORDER BY created_at_ns")?;
    let rows = s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn latest_param(conn: &Connection, project_id: &str) -> DomainResult<Option<(String, serde_json::Value, String)>> {
    let mut s = conn.prepare(
        "SELECT id,payload,fingerprint FROM params WHERE project_id=?1 ORDER BY seq DESC LIMIT 1",
    )?;
    let mut rows = s.query_map(params![project_id], |r| {
        let payload: String = r.get(1)?;
        Ok((r.get::<_, String>(0)?, serde_json::from_str(&payload).unwrap(), r.get::<_, String>(2)?))
    })?;
    if let Some(x) = rows.next() {
        return Ok(Some(x?));
    }
    Ok(None)
}

pub fn insert_param(conn: &Connection, project_id: &str, id: &str, seq: i64, parent: Option<&str>, payload: &serde_json::Value, fp: &str, event_id: &str) -> DomainResult<()> {
    conn.execute(
        "INSERT INTO params(id,project_id,seq,parent_id,payload,fingerprint,created_event)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id, project_id, seq, parent, serde_json::to_string(payload)?, fp, event_id],
    )?;
    Ok(())
}

pub fn insert_result(conn: &Connection, project_id: &str, id: &str, kind: &str, sources: &[String], payload: &serde_json::Value, event_id: &str, seq: i64, fp: &str) -> DomainResult<()> {
    conn.execute(
        "INSERT INTO results(id,project_id,kind,algo_version,param_fingerprint,sources,payload,created_event,created_seq,superseded_event)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL)",
        params![id, project_id, kind, ALGO_VERSION, fp, serde_json::to_string(sources)?, serde_json::to_string(payload)?, event_id, seq],
    )?;
}

pub fn latest_result(conn: &Connection, project_id: &str, kind: &str) -> DomainResult<Option<(String, serde_json::Value, String, i64)>> {
    let mut s = conn.prepare(
        "SELECT id,payload,param_fingerprint,created_seq FROM results
         WHERE project_id=?1 AND kind=?2 ORDER BY created_seq DESC LIMIT 1",
    )?;
    let mut rows = s.query_map(params![project_id, kind], |r| {
        let payload: String = r.get(1)?;
        Ok((r.get::<_, String>(0)?, serde_json::from_str(&payload).unwrap(), r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    if let Some(x) = rows.next() {
        return Ok(Some(x?));
    }
    Ok(None)
}

/// Result visible at an old event (as-of replay): the most recent result whose
/// creating seq <= at_seq. Superseded values remain readable but are flagged by
/// the caller because a newer result exists.
pub fn result_as_of(conn: &Connection, project_id: &str, kind: &str, at_seq: i64) -> DomainResult<Option<(String, serde_json::Value, bool)>> {
    let mut s = conn.prepare(
        "SELECT id,payload,(SELECT MAX(created_seq) FROM results r2
            WHERE r2.project_id=results.project_id AND r2.kind=results.kind) AS latest_seq,
            created_seq FROM results
         WHERE project_id=?1 AND kind=?2 AND created_seq<=?3 ORDER BY created_seq DESC LIMIT 1",
    )?;
    let mut rows = s.query_map(params![project_id, kind, at_seq], |r| {
        let payload: String = r.get(1)?;
        let latest: i64 = r.get(2)?;
        let created: i64 = r.get(3)?;
        Ok((r.get::<_, String>(0)?, serde_json::from_str(&payload).unwrap(), created < latest))
    })?;
    if let Some(x) = rows.next() {
        return Ok(Some(x?));
    }
    Ok(None)
}

pub fn insert_decision(conn: &Connection, project_id: &str, id: &str, target: &str, resolution: &str, rationale: &str, event_id: &str, seq: i64) -> DomainResult<()> {
    conn.execute(
        "INSERT INTO decisions(id,project_id,target_result_id,resolution,rationale,event_id,seq)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id, project_id, target, resolution, rationale, event_id, seq],
    )?;
    Ok(())
}

pub fn decisions_for(conn: &Connection, project_id: &str, result_id: &str) -> DomainResult<Vec<(String, String, String, i64)>> {
    let mut s = conn.prepare(
        "SELECT id,resolution,rationale,seq FROM decisions WHERE project_id=?1 AND target_result_id=?2 ORDER BY seq",
    )?;
    let rows = s.query_map(params![project_id, result_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn mark_superseded(conn: &Connection, project_id: &str, kind: &str, new_event_id: &str) -> DomainResult<()> {
    conn.execute(
        "UPDATE results SET superseded_event=?1 WHERE project_id=?2 AND kind=?3 AND superseded_event IS NULL",
        params![new_event_id, project_id, kind],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn ensure_err<T>(e: DomainError) -> DomainError {
    e
}
