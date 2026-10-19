//! Durable journal for reversible per-torrent transfer-owner transitions.
//!
//! The journal never grants concurrent ownership. A cutover is recorded as
//! non-stable before the active actor is drained; after a process restart any
//! unfinished transition is fenced back to the reference owner.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use thiserror::Error;

const MAX_INFO_HASH_BYTES: usize = 64;
const MAX_FAILURE_REASON_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferOwner {
    Reference,
    SharedCore,
}

impl TransferOwner {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::SharedCore => "shared_core",
        }
    }

    fn parse(value: String) -> rusqlite::Result<Self> {
        match value.as_str() {
            "reference" => Ok(Self::Reference),
            "shared_core" => Ok(Self::SharedCore),
            _ => Err(rusqlite::Error::InvalidColumnType(
                0,
                "owner_kind".to_owned(),
                rusqlite::types::Type::Text,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferCutoverPhase {
    Stable,
    Draining,
    Starting,
    RollbackRequired,
}

impl TransferCutoverPhase {
    fn parse(value: String) -> rusqlite::Result<Self> {
        match value.as_str() {
            "stable" => Ok(Self::Stable),
            "draining" => Ok(Self::Draining),
            "starting" => Ok(Self::Starting),
            "rollback_required" => Ok(Self::RollbackRequired),
            _ => Err(rusqlite::Error::InvalidColumnType(
                0,
                "phase".to_owned(),
                rusqlite::types::Type::Text,
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferCutoverRow {
    pub info_hash: String,
    pub owner_generation: i64,
    pub owner: TransferOwner,
    pub target: TransferOwner,
    pub phase: TransferCutoverPhase,
    pub failure_reason: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum TransferCutoverError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid transfer cutover input: {0}")]
    Invalid(String),
    #[error("transfer cutover generation conflict for {0}")]
    Conflict(String),
}

fn validate(info_hash: &str, updated_at: i64) -> Result<(), TransferCutoverError> {
    if info_hash.is_empty() || info_hash.len() > MAX_INFO_HASH_BYTES {
        return Err(TransferCutoverError::Invalid(
            "info hash must contain between 1 and 64 bytes".to_owned(),
        ));
    }
    if updated_at < 0 {
        return Err(TransferCutoverError::Invalid(
            "updated_at must be non-negative".to_owned(),
        ));
    }
    Ok(())
}

fn decode(
    value: (String, i64, String, String, String, Option<String>, i64),
) -> rusqlite::Result<TransferCutoverRow> {
    Ok(TransferCutoverRow {
        info_hash: value.0,
        owner_generation: value.1,
        owner: TransferOwner::parse(value.2)?,
        target: TransferOwner::parse(value.3)?,
        phase: TransferCutoverPhase::parse(value.4)?,
        failure_reason: value.5,
        updated_at: value.6,
    })
}

fn query(conn: &Connection, info_hash: &str) -> rusqlite::Result<Option<TransferCutoverRow>> {
    let row = conn
        .query_row(
            "SELECT info_hash, owner_generation, owner_kind, target_kind, phase,
                    failure_reason, updated_at
             FROM transfer_cutovers WHERE info_hash = ?1",
            [info_hash],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    row.map(decode).transpose()
}

pub fn get(
    conn: &Connection,
    info_hash: &str,
) -> Result<Option<TransferCutoverRow>, TransferCutoverError> {
    validate(info_hash, 0)?;
    Ok(query(conn, info_hash)?)
}

/// Create the stable owner record on first use without changing an existing
/// route. This makes the initial candidate/reference selection durable too.
pub fn ensure_owner(
    conn: &mut Connection,
    info_hash: &str,
    owner: TransferOwner,
    updated_at: i64,
) -> Result<TransferCutoverRow, TransferCutoverError> {
    validate(info_hash, updated_at)?;
    conn.execute(
        "INSERT OR IGNORE INTO transfer_cutovers
            (info_hash, owner_generation, owner_kind, target_kind, phase, failure_reason, updated_at)
         VALUES (?1, 0, ?2, ?2, 'stable', NULL, ?3)",
        params![info_hash, owner.as_str(), updated_at],
    )?;
    query(conn, info_hash)?.ok_or_else(|| {
        TransferCutoverError::Invalid("owner row disappeared after ensure".to_owned())
    })
}

/// Restore the recorded owner after a failed transition. The owner is never
/// changed by this operation; only the target and phase return to stable.
pub fn finish_rollback(
    conn: &mut Connection,
    info_hash: &str,
    generation: i64,
    reason: &str,
    updated_at: i64,
) -> Result<(), TransferCutoverError> {
    validate(info_hash, updated_at)?;
    if reason.is_empty() || reason.len() > MAX_FAILURE_REASON_BYTES {
        return Err(TransferCutoverError::Invalid(
            "failure reason must contain between 1 and 4096 bytes".to_owned(),
        ));
    }
    let changed = conn.execute(
        "UPDATE transfer_cutovers
         SET target_kind = owner_kind, phase = 'stable', failure_reason = ?1, updated_at = ?2
         WHERE info_hash = ?3 AND owner_generation = ?4
           AND phase IN ('draining', 'starting', 'rollback_required')",
        params![reason, updated_at, info_hash, generation],
    )?;
    if changed != 1 {
        return Err(TransferCutoverError::Conflict(info_hash.to_owned()));
    }
    Ok(())
}

/// Persist the transition intent and advance its fencing generation.
pub fn begin(
    conn: &mut Connection,
    info_hash: &str,
    expected_generation: i64,
    target: TransferOwner,
    updated_at: i64,
) -> Result<TransferCutoverRow, TransferCutoverError> {
    validate(info_hash, updated_at)?;
    if expected_generation < 0 || expected_generation == i64::MAX {
        return Err(TransferCutoverError::Invalid(
            "expected generation must be a non-negative incrementable integer".to_owned(),
        ));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR IGNORE INTO transfer_cutovers
            (info_hash, owner_generation, owner_kind, target_kind, phase, failure_reason, updated_at)
         VALUES (?1, 0, 'reference', 'reference', 'stable', NULL, ?2)",
        params![info_hash, updated_at],
    )?;
    let changed = tx.execute(
        "UPDATE transfer_cutovers
         SET owner_generation = owner_generation + 1,
             target_kind = ?1,
             phase = 'draining',
             failure_reason = NULL,
             updated_at = ?2
         WHERE info_hash = ?3 AND owner_generation = ?4 AND phase = 'stable'
           AND owner_kind != ?1",
        params![target.as_str(), updated_at, info_hash, expected_generation],
    )?;
    if changed != 1 {
        return Err(TransferCutoverError::Conflict(info_hash.to_owned()));
    }
    let row = query(&tx, info_hash)?.ok_or_else(|| {
        TransferCutoverError::Invalid("journal row disappeared during begin".to_owned())
    })?;
    tx.commit()?;
    Ok(row)
}

/// Record that all old-owner work has drained; the target may now start.
pub fn mark_drained(
    conn: &mut Connection,
    info_hash: &str,
    generation: i64,
    updated_at: i64,
) -> Result<(), TransferCutoverError> {
    validate(info_hash, updated_at)?;
    let changed = conn.execute(
        "UPDATE transfer_cutovers SET phase = 'starting', updated_at = ?1
         WHERE info_hash = ?2 AND owner_generation = ?3 AND phase = 'draining'",
        params![updated_at, info_hash, generation],
    )?;
    if changed != 1 {
        return Err(TransferCutoverError::Conflict(info_hash.to_owned()));
    }
    Ok(())
}

/// Publish the new owner only after its actor has started successfully.
pub fn commit(
    conn: &mut Connection,
    info_hash: &str,
    generation: i64,
    updated_at: i64,
) -> Result<(), TransferCutoverError> {
    validate(info_hash, updated_at)?;
    let changed = conn.execute(
        "UPDATE transfer_cutovers
         SET owner_kind = target_kind, phase = 'stable', failure_reason = NULL, updated_at = ?1
         WHERE info_hash = ?2 AND owner_generation = ?3 AND phase = 'starting'",
        params![updated_at, info_hash, generation],
    )?;
    if changed != 1 {
        return Err(TransferCutoverError::Conflict(info_hash.to_owned()));
    }
    Ok(())
}

pub fn require_rollback(
    conn: &mut Connection,
    info_hash: &str,
    generation: i64,
    reason: &str,
    updated_at: i64,
) -> Result<(), TransferCutoverError> {
    validate(info_hash, updated_at)?;
    if reason.is_empty() || reason.len() > MAX_FAILURE_REASON_BYTES {
        return Err(TransferCutoverError::Invalid(
            "failure reason must contain between 1 and 4096 bytes".to_owned(),
        ));
    }
    let changed = conn.execute(
        "UPDATE transfer_cutovers
         SET phase = 'rollback_required', failure_reason = ?1, updated_at = ?2
         WHERE info_hash = ?3 AND owner_generation = ?4
           AND phase IN ('draining', 'starting')",
        params![reason, updated_at, info_hash, generation],
    )?;
    if changed != 1 {
        return Err(TransferCutoverError::Conflict(info_hash.to_owned()));
    }
    Ok(())
}

/// Recover incomplete cutovers before any torrent actor is restored.
/// Every non-stable journal is fenced to the reference route in one commit.
pub fn recover_interrupted(
    conn: &mut Connection,
    updated_at: i64,
) -> Result<usize, TransferCutoverError> {
    validate("startup", updated_at)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE transfer_cutovers
         SET owner_generation = owner_generation + 1,
             owner_kind = 'reference', target_kind = 'reference', phase = 'stable',
             failure_reason = COALESCE(failure_reason, 'interrupted cutover recovered to reference owner'),
             updated_at = ?1
         WHERE phase != 'stable'",
        [updated_at],
    )?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::migrate;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO torrents (info_hash, name, total_length, piece_length, piece_count,
                                   save_path, added_at)
             VALUES ('0123456789abcdef', 'fixture', 1, 1, 1, '/tmp', 1)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn cutover_requires_drain_before_commit_and_increments_generation() {
        let mut conn = database();
        let intent = begin(
            &mut conn,
            "0123456789abcdef",
            0,
            TransferOwner::SharedCore,
            2,
        )
        .unwrap();
        assert_eq!(intent.owner_generation, 1);
        assert_eq!(intent.owner, TransferOwner::Reference);
        assert_eq!(intent.target, TransferOwner::SharedCore);
        assert_eq!(intent.phase, TransferCutoverPhase::Draining);
        assert!(commit(&mut conn, "0123456789abcdef", 1, 3).is_err());

        mark_drained(&mut conn, "0123456789abcdef", 1, 3).unwrap();
        commit(&mut conn, "0123456789abcdef", 1, 4).unwrap();
        let stable = get(&conn, "0123456789abcdef").unwrap().unwrap();
        assert_eq!(stable.owner, TransferOwner::SharedCore);
        assert_eq!(stable.target, TransferOwner::SharedCore);
        assert_eq!(stable.phase, TransferCutoverPhase::Stable);
    }

    #[test]
    fn initial_owner_is_stable_and_failed_transition_restores_it() {
        let mut conn = database();
        let initial =
            ensure_owner(&mut conn, "0123456789abcdef", TransferOwner::SharedCore, 2).unwrap();
        assert_eq!(initial.owner, TransferOwner::SharedCore);
        assert_eq!(initial.target, TransferOwner::SharedCore);
        assert_eq!(initial.phase, TransferCutoverPhase::Stable);

        let existing =
            ensure_owner(&mut conn, "0123456789abcdef", TransferOwner::Reference, 3).unwrap();
        assert_eq!(existing.owner, TransferOwner::SharedCore);

        let intent = begin(
            &mut conn,
            "0123456789abcdef",
            initial.owner_generation,
            TransferOwner::Reference,
            4,
        )
        .unwrap();
        mark_drained(&mut conn, "0123456789abcdef", intent.owner_generation, 5).unwrap();
        require_rollback(
            &mut conn,
            "0123456789abcdef",
            intent.owner_generation,
            "reference actor failed to start",
            6,
        )
        .unwrap();
        finish_rollback(
            &mut conn,
            "0123456789abcdef",
            intent.owner_generation,
            "candidate owner restored",
            7,
        )
        .unwrap();

        let stable = get(&conn, "0123456789abcdef").unwrap().unwrap();
        assert_eq!(stable.owner_generation, 1);
        assert_eq!(stable.owner, TransferOwner::SharedCore);
        assert_eq!(stable.target, TransferOwner::SharedCore);
        assert_eq!(stable.phase, TransferCutoverPhase::Stable);
    }

    #[test]
    fn generation_cas_rejects_stale_transition_and_interrupted_startup_rolls_back() {
        let mut conn = database();
        begin(
            &mut conn,
            "0123456789abcdef",
            0,
            TransferOwner::SharedCore,
            2,
        )
        .unwrap();
        assert!(begin(
            &mut conn,
            "0123456789abcdef",
            0,
            TransferOwner::SharedCore,
            3
        )
        .is_err());
        mark_drained(&mut conn, "0123456789abcdef", 1, 4).unwrap();
        require_rollback(&mut conn, "0123456789abcdef", 1, "candidate failed", 5).unwrap();

        assert_eq!(recover_interrupted(&mut conn, 6).unwrap(), 1);
        let recovered = get(&conn, "0123456789abcdef").unwrap().unwrap();
        assert_eq!(recovered.owner_generation, 2);
        assert_eq!(recovered.owner, TransferOwner::Reference);
        assert_eq!(recovered.target, TransferOwner::Reference);
        assert_eq!(recovered.phase, TransferCutoverPhase::Stable);
        assert_eq!(
            recovered.failure_reason.as_deref(),
            Some("candidate failed")
        );
        assert_eq!(recover_interrupted(&mut conn, 7).unwrap(), 0);
    }
}
