use rusqlite::Connection;

const INITIAL_MIGRATION: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations/001_initial.sql"));

pub fn open_and_migrate(path: &str) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    conn.execute_batch(INITIAL_MIGRATION)?;
    Ok(conn)
}
