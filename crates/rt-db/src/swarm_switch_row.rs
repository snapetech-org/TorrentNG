//! A durable, exclusive relationship between an untouched private torrent and
//! its public counterpart. Non-public phases never authorize network activity.
use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SwarmSwitchRow {
    pub original_hash: String,
    pub public_hash: String,
    pub phase: String,
    pub updated_at: i64,
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<SwarmSwitchRow>> {
    conn.prepare("SELECT original_hash, public_hash, phase, updated_at FROM swarm_switches")?
        .query_map([], |r| {
            Ok(SwarmSwitchRow {
                original_hash: r.get(0)?,
                public_hash: r.get(1)?,
                phase: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })?
        .collect()
}

pub fn begin(conn: &mut Connection, row: &SwarmSwitchRow) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    // Both identities are reserved before the public torrent can be created.
    tx.execute("INSERT INTO swarm_switches(original_hash, public_hash, phase, updated_at) VALUES (?1, ?2, 'preparing', ?3)",
        params![row.original_hash, row.public_hash, row.updated_at])?;
    tx.commit()
}

pub fn set_phase(conn: &Connection, original: &str, phase: &str, now: i64) -> rusqlite::Result<()> {
    if conn.execute(
        "UPDATE swarm_switches SET phase = ?2, updated_at = ?3 WHERE original_hash = ?1",
        params![original, phase, now],
    )? != 1
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// Called before any actor is restored. Interrupted transitions stay parked
/// until Restore private swarm is retried; stable public sessions retain state.
pub fn recover(conn: &mut Connection) -> rusqlite::Result<Vec<SwarmSwitchRow>> {
    let tx = conn.transaction()?;
    tx.execute("UPDATE torrents SET state = 'paused' WHERE info_hash IN
        (SELECT original_hash FROM swarm_switches UNION SELECT public_hash FROM swarm_switches WHERE phase != 'public')", [])?;
    let rows = list(&tx)?;
    tx.commit()?;
    Ok(rows)
}

/// This is the final restore commit, after public I/O has stopped. Keeping the
/// original in Checking across a crash prevents stale private fastresume reuse.
pub fn finish(conn: &mut Connection, original: &str, public: &str) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let removed = tx.execute("DELETE FROM swarm_switches WHERE original_hash = ?1 AND public_hash = ?2 AND phase = 'restoring'",
        params![original, public])?;
    if removed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    tx.execute("DELETE FROM torrents WHERE info_hash = ?1", [public])?;
    if tx.execute(
        "UPDATE torrents SET state = 'checking' WHERE info_hash = ?1",
        [original],
    )? != 1
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_switch_parks_both_and_finish_is_atomic() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        for hash in ["private", "public"] {
            conn.execute("INSERT INTO torrents(info_hash,name,total_length,piece_length,piece_count,save_path,added_at,state)
                VALUES (?1,'fixture',4,16384,1,'/tmp',0,'downloading')", [hash]).unwrap();
        }
        let row = SwarmSwitchRow {
            original_hash: "private".into(),
            public_hash: "public".into(),
            phase: "preparing".into(),
            updated_at: 0,
        };
        begin(&mut conn, &row).unwrap();
        assert!(finish(&mut conn, "private", "public").is_err());
        assert!(crate::get(&conn, "public").is_ok());
        recover(&mut conn).unwrap();
        for hash in ["private", "public"] {
            assert_eq!(crate::get(&conn, hash).unwrap().state, "paused");
        }
        set_phase(&conn, "private", "public", 1).unwrap();
        conn.execute(
            "UPDATE torrents SET state='downloading' WHERE info_hash='public'",
            [],
        )
        .unwrap();
        recover(&mut conn).unwrap();
        assert_eq!(crate::get(&conn, "public").unwrap().state, "downloading");
        set_phase(&conn, "private", "restoring", 2).unwrap();
        finish(&mut conn, "private", "public").unwrap();
        assert!(list(&conn).unwrap().is_empty());
        assert!(crate::get(&conn, "public").is_err());
        assert_eq!(crate::get(&conn, "private").unwrap().state, "checking");
    }
}
