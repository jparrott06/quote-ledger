//! SQLite persistence for append-only `StoredEvent` rows + idempotency keys.

use prost::Message;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::error::StoreError;
use crate::v1::{QuoteEvent, StoredEvent};

fn row_to_stored(seq: i64, quote_id: String, payload: Vec<u8>) -> Result<StoredEvent, StoreError> {
    let event =
        QuoteEvent::decode(payload.as_slice()).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    Ok(StoredEvent {
        seq: seq as u64,
        quote_id,
        event: Some(event),
    })
}

pub fn load_stored_events(
    conn: &Connection,
    quote_id: &str,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT seq, quote_id, payload FROM events WHERE quote_id = ?1 ORDER BY seq ASC",
    )?;
    let mut rows = stmt.query(params![quote_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let qid: String = row.get(1)?;
        let payload: Vec<u8> = row.get(2)?;
        out.push(row_to_stored(seq, qid, payload)?);
    }
    Ok(out)
}

pub fn try_load_idempotent_events(
    conn: &Connection,
    quote_id: &str,
    client_command_id: &str,
) -> Result<Option<Vec<StoredEvent>>, StoreError> {
    let Some((first, last)) = idempotency_lookup(conn, quote_id, client_command_id)? else {
        return Ok(None);
    };
    Ok(Some(load_events_in_seq_range(conn, quote_id, first, last)?))
}

pub fn idempotency_lookup(
    conn: &Connection,
    quote_id: &str,
    client_command_id: &str,
) -> Result<Option<(u64, u64)>, StoreError> {
    let row = conn
        .query_row(
            "SELECT first_seq, last_seq FROM idempotency_keys WHERE quote_id = ?1 AND client_command_id = ?2",
            params![quote_id, client_command_id],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )
        .optional()?;
    Ok(row)
}

fn load_events_in_seq_range(
    conn: &Connection,
    quote_id: &str,
    first_seq: u64,
    last_seq: u64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT seq, quote_id, payload FROM events WHERE quote_id = ?1 AND seq BETWEEN ?2 AND ?3 ORDER BY seq ASC",
    )?;
    let mut rows = stmt.query(params![quote_id, first_seq as i64, last_seq as i64])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let qid: String = row.get(1)?;
        let payload: Vec<u8> = row.get(2)?;
        out.push(row_to_stored(seq, qid, payload)?);
    }
    Ok(out)
}

fn event_type_for(ev: &QuoteEvent) -> &'static str {
    match ev.kind {
        Some(crate::v1::quote_event::Kind::QuoteCreated(_)) => "quote_created",
        Some(crate::v1::quote_event::Kind::LineItemAdded(_)) => "line_item_added",
        Some(crate::v1::quote_event::Kind::QuoteFinalized(_)) => "quote_finalized",
        None => "unknown",
    }
}

/// Append new events for a command, or return previously stored events if idempotent replay.
pub fn append_command_events(
    conn: &mut Connection,
    quote_id: &str,
    client_command_id: &str,
    events: &[QuoteEvent],
) -> Result<Vec<StoredEvent>, StoreError> {
    if let Some((first, last)) = idempotency_lookup(conn, quote_id, client_command_id)? {
        return load_events_in_seq_range(conn, quote_id, first, last);
    }

    if events.is_empty() {
        return Err(StoreError::EmptyAppend);
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let next_seq: u64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE quote_id = ?1",
        params![quote_id],
        |r| Ok(r.get::<_, i64>(0)? as u64),
    )?;

    let mut seq_cursor = next_seq;
    let first_seq = seq_cursor;
    let mut stored = Vec::new();

    for ev in events {
        let mut payload = Vec::new();
        ev.encode(&mut payload)
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;

        tx.execute(
            "INSERT INTO events (quote_id, seq, event_type, payload) VALUES (?1, ?2, ?3, ?4)",
            params![quote_id, seq_cursor as i64, event_type_for(ev), payload,],
        )?;

        stored.push(StoredEvent {
            seq: seq_cursor,
            quote_id: quote_id.to_string(),
            event: Some(ev.clone()),
        });

        seq_cursor += 1;
    }

    let last_seq = seq_cursor - 1;

    tx.execute(
        "INSERT INTO idempotency_keys (quote_id, client_command_id, first_seq, last_seq) VALUES (?1, ?2, ?3, ?4)",
        params![
            quote_id,
            client_command_id,
            first_seq as i64,
            last_seq as i64,
        ],
    )?;

    tx.commit()?;
    Ok(stored)
}

pub fn load_stored_events_between(
    conn: &Connection,
    quote_id: &str,
    after_seq_exclusive: u64,
    upto_seq_inclusive: u64,
) -> Result<Vec<StoredEvent>, StoreError> {
    if upto_seq_inclusive <= after_seq_exclusive {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT seq, quote_id, payload FROM events WHERE quote_id = ?1 AND seq > ?2 AND seq <= ?3 ORDER BY seq ASC",
    )?;
    let mut rows = stmt.query(params![
        quote_id,
        after_seq_exclusive as i64,
        upto_seq_inclusive as i64
    ])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let qid: String = row.get(1)?;
        let payload: Vec<u8> = row.get(2)?;
        out.push(row_to_stored(seq, qid, payload)?);
    }
    Ok(out)
}
