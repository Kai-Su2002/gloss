use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex, time::Duration};

/// One connection serializes writes. IDs are opaque keys, never filesystem paths.
pub struct Storage(Mutex<Connection>);

impl Storage {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS documents (id TEXT PRIMARY KEY NOT NULL, body TEXT);
                 CREATE TABLE IF NOT EXISTS pdfs (
                   id TEXT PRIMARY KEY NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                   data BLOB NOT NULL
                 );",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self(Mutex::new(connection)))
    }

    fn with_connection<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut connection = self
            .0
            .lock()
            .map_err(|_| "Storage lock poisoned".to_owned())?;
        action(&mut connection).map_err(|e| e.to_string())
    }

    pub fn load_state(&self, key: &str) -> Result<Option<String>, String> {
        self.with_connection(|c| {
            c.query_row("SELECT value FROM state WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
        })
    }

    pub fn save_state(&self, key: &str, value: &str) -> Result<(), String> {
        self.with_connection(|c| {
            c.execute("INSERT INTO state(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
            Ok(())
        })
    }

    pub fn load_document(&self, id: &str) -> Result<Option<String>, String> {
        self.with_connection(|c| {
            c.query_row("SELECT body FROM documents WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn save_document(&self, id: &str, document: &str) -> Result<(), String> {
        self.with_connection(|c| {
            // Never REPLACE: that would delete the row and cascade away the PDF.
            c.execute("INSERT INTO documents(id, body) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET body = excluded.body", params![id, document])?;
            Ok(())
        })
    }

    pub fn save_pdf(&self, id: &str, data: &[u8]) -> Result<(), String> {
        self.with_connection(|c| {
            let transaction = c.transaction()?;
            transaction.execute("INSERT INTO documents(id) VALUES (?1) ON CONFLICT(id) DO NOTHING", [id])?;
            transaction.execute("INSERT INTO pdfs(id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = excluded.data", params![id, data])?;
            transaction.commit()
        })
    }

    pub fn load_pdf(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        self.with_connection(|c| {
            c.query_row("SELECT data FROM pdfs WHERE id = ?1", [id], |r| r.get(0))
                .optional()
        })
    }

    pub fn delete_document(&self, id: &str) -> Result<(), String> {
        self.with_connection(|c| {
            c.execute("DELETE FROM documents WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// Keep the identity row (and its PDF) available for a future re-parse.
    pub fn forget_document(&self, id: &str) -> Result<(), String> {
        self.with_connection(|c| {
            c.execute("UPDATE documents SET body = NULL WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}
