use crate::core::{self, Direction, Group};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
    mpsc::{self, SyncSender},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

const SCHEMA: &str = "PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS sessions(
 id INTEGER PRIMARY KEY, group_name TEXT NOT NULL, port TEXT NOT NULL,
 line_settings TEXT NOT NULL, outputs TEXT NOT NULL, started_at TEXT NOT NULL,
 ended_at TEXT, end_reason TEXT);
CREATE TABLE IF NOT EXISTS chunks(
 id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id),
 utc TEXT NOT NULL, mono_us INTEGER NOT NULL,
 direction TEXT NOT NULL CHECK(direction IN ('RX','TX')), source TEXT, raw BLOB NOT NULL);
CREATE INDEX IF NOT EXISTS chunks_session ON chunks(session_id,id);
CREATE INDEX IF NOT EXISTS chunks_utc ON chunks(utc);";

pub enum Record {
    Begin {
        key: u64,
        group: Group,
        utc: String,
    },
    Chunk {
        key: u64,
        utc: String,
        mono_us: u64,
        direction: Direction,
        source: Option<String>,
        raw: Vec<u8>,
    },
    End {
        key: u64,
        utc: String,
        reason: String,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct Sink {
    tx: SyncSender<Record>,
    pub error: Arc<Mutex<Option<String>>>,
}

impl Sink {
    pub fn send(&self, record: Record) {
        if self.tx.send(record).is_err() {
            *self.error.lock().unwrap() = Some("录制线程已退出，数据未录入数据库".into());
        }
    }
}

pub struct Recorder {
    pub sink: Sink,
    pub retention: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}

fn recover(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("UPDATE sessions SET ended_at=COALESCE((SELECT utc FROM chunks WHERE session_id=sessions.id ORDER BY id DESC LIMIT 1),started_at), end_reason='程序异常退出' WHERE ended_at IS NULL", [])?;
    Ok(())
}

fn prune(conn: &mut Connection, days: u32) -> rusqlite::Result<()> {
    let cutoff = core::retention_cutoff(core::epoch_ms(SystemTime::now()), days);
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM chunks WHERE utc < ?1", [&cutoff])?;
    tx.execute("DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1 AND NOT EXISTS(SELECT 1 FROM chunks WHERE session_id=sessions.id)", [&cutoff])?;
    tx.commit()
}

fn commit(
    conn: &mut Connection,
    pending: &mut Vec<Record>,
    sessions: &mut HashMap<u64, i64>,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    // Only publish the key mapping after the transaction commits.
    let mut next = sessions.clone();
    for msg in pending.iter() {
        match msg {
            Record::Begin { key, group, utc } => {
                tx.execute("INSERT INTO sessions(group_name,port,line_settings,outputs,started_at) VALUES(?1,?2,?3,?4,?5)", params![group.name, group.port, group.line_settings(), group.outputs.join(","), utc])?;
                next.insert(*key, tx.last_insert_rowid());
            }
            Record::Chunk {
                key,
                utc,
                mono_us,
                direction,
                source,
                raw,
            } => {
                let id = next.get(key).ok_or(rusqlite::Error::InvalidQuery)?;
                tx.execute("INSERT INTO chunks(session_id,utc,mono_us,direction,source,raw) VALUES(?1,?2,?3,?4,?5,?6)", params![id, utc, *mono_us as i64, direction.key(), source, raw])?;
            }
            Record::End { key, utc, reason } => {
                if let Some(id) = next.remove(key) {
                    tx.execute(
                        "UPDATE sessions SET ended_at=?1,end_reason=?2 WHERE id=?3",
                        params![utc, reason, id],
                    )?;
                }
            }
            Record::Shutdown => {}
        }
    }
    tx.commit()?;
    *sessions = next;
    pending.clear();
    Ok(())
}

impl Recorder {
    pub fn start(path: &Path, days: u32) -> Self {
        let (tx, rx) = mpsc::sync_channel(2048);
        let error = Arc::new(Mutex::new(None));
        let retention = Arc::new(AtomicU32::new(days));
        let sink = Sink {
            tx,
            error: error.clone(),
        };
        let path = path.to_owned();
        let keep = retention.clone();
        let handle = thread::spawn(move || {
            let open = || -> rusqlite::Result<Connection> {
                let mut conn = Connection::open(&path)?;
                conn.busy_timeout(Duration::from_secs(2))?;
                conn.execute_batch(SCHEMA)?;
                recover(&conn)?;
                prune(&mut conn, keep.load(Ordering::Relaxed))?;
                Ok(conn)
            };
            let mut conn = match open() {
                Ok(c) => Some(c),
                Err(e) => {
                    *error.lock().unwrap() =
                        Some(format!("数据库无法打开：{e}；转发继续，录制不可用"));
                    None
                }
            };
            let mut pending = Vec::new();
            let mut sessions = HashMap::new();
            let mut last_commit = Instant::now();
            let mut last_prune = Instant::now();
            loop {
                let msg = rx.recv_timeout(Duration::from_millis(20));
                let shutdown = matches!(
                    msg,
                    Ok(Record::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected)
                );
                if let Ok(r) = msg {
                    pending.push(r);
                }
                if pending.len() >= 500
                    || last_commit.elapsed() >= Duration::from_millis(200)
                    || shutdown
                {
                    if !pending.is_empty() {
                        if let Some(c) = conn.as_mut() {
                            if let Err(e) = commit(c, &mut pending, &mut sessions) {
                                *error.lock().unwrap() =
                                    Some(format!("录制写库失败：{e}；此后录制不可用，转发继续"));
                                conn = None;
                            }
                        }
                        // A failed recorder keeps draining so serial forwarding remains available.
                        // The persistent red error is deliberately never cleared by display refresh.
                        if conn.is_none() {
                            pending.clear();
                        }
                    }
                    last_commit = Instant::now();
                }
                if shutdown {
                    break;
                }
                if last_prune.elapsed() >= Duration::from_secs(6 * 3600) {
                    if let Some(c) = conn.as_mut() {
                        if let Err(e) = prune(c, keep.load(Ordering::Relaxed)) {
                            *error.lock().unwrap() = Some(format!("数据库过期清理失败：{e}"));
                        }
                    }
                    last_prune = Instant::now();
                }
            }
        });
        Self {
            sink,
            retention,
            handle: Some(handle),
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(h) = self.handle.take() {
            self.sink.send(Record::Shutdown);
            let _ = h.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_recovery_and_retention() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let g = Group::new(1, "测试".into());
        let mut map = HashMap::new();
        let mut p = vec![
            Record::Begin {
                key: 1,
                group: g.clone(),
                utc: "2000-01-01T00:00:00.000Z".into(),
            },
            Record::Chunk {
                key: 1,
                utc: "2000-01-01T00:00:01.000Z".into(),
                mono_us: 123,
                direction: Direction::Tx,
                source: Some("COM10".into()),
                raw: vec![0, 255],
            },
        ];
        commit(&mut c, &mut p, &mut map).unwrap();
        let raw: Vec<u8> = c
            .query_row("SELECT raw FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raw, vec![0, 255]);
        recover(&c).unwrap();
        let (end, why): (String, String) = c
            .query_row("SELECT ended_at,end_reason FROM sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(end, "2000-01-01T00:00:01.000Z");
        assert_eq!(why, "程序异常退出");
        p.push(Record::Begin {
            key: 2,
            group: g,
            utc: core::utc_iso(core::epoch_ms(SystemTime::now())),
        });
        commit(&mut c, &mut p, &mut map).unwrap();
        prune(&mut c, 30).unwrap();
        let count: i64 = c
            .query_row("SELECT count(*) FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = c
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1); // Active sessions are never pruned.
        p.push(Record::End {
            key: 2,
            utc: "2026-10-14T00:00:00.000Z".into(),
            reason: "用户停止".into(),
        });
        commit(&mut c, &mut p, &mut map).unwrap();
        assert!(map.get(&2).is_none());
    }
    #[test]
    fn startup_failure_is_visible_and_queue_keeps_draining() {
        let path = std::env::current_dir()
            .unwrap()
            .join("target/this-directory-does-not-exist/capture.sqlite3");
        let mut recorder = Recorder::start(&path, 30);
        for key in 0..3000 {
            recorder.sink.send(Record::End {
                key,
                utc: "now".into(),
                reason: "test".into(),
            });
        }
        recorder.shutdown();
        assert!(
            recorder
                .sink
                .error
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("数据库无法打开")
        );
    }
    #[test]
    fn empty_abandoned_session_ends_at_start() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let mut pending = vec![Record::Begin {
            key: 7,
            group: Group::new(7, "empty".into()),
            utc: "2026-10-14T00:00:00.000Z".into(),
        }];
        commit(&mut c, &mut pending, &mut HashMap::new()).unwrap();
        recover(&c).unwrap();
        let (start, end): (String, String) = c
            .query_row("SELECT started_at,ended_at FROM sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(start, end);
    }
}
