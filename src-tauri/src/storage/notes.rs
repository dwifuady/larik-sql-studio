// Sticky Notes storage (v2 — gutter icon approach)
// Stores notes in the SQLite database, keyed by tab_id.
// Notes are completely decoupled from SQL editor content.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::database::{DatabaseManager, StorageResult};

/// A sticky note attached to a specific line in a tab's editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickyNote {
    pub id: String,
    pub tab_id: String,
    pub line_number: i32,
    pub content: String,
    /// Semantic color key: "yellow", "blue", "green", "orange", "pink", "purple"
    pub color: String,
    pub minimized: bool,
    /// Persisted popover width in pixels (None = use default).
    #[serde(default)]
    pub width: Option<i32>,
    /// When true the popover stays open on outside click.
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl DatabaseManager {
    /// Get all sticky notes for a given tab.
    pub fn get_tab_notes(&self, tab_id: &str) -> StorageResult<Vec<StickyNote>> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, tab_id, line_number, content, color, minimized, width, pinned, created_at, updated_at
                 FROM sticky_notes
                 WHERE tab_id = ?1
                 ORDER BY line_number ASC",
            )?;

            let notes = stmt
                .query_map(params![tab_id], |row| {
                    let minimized: i32 = row.get(5)?;
                    let pinned: i32 = row.get(7)?;
                    Ok(StickyNote {
                        id: row.get(0)?,
                        tab_id: row.get(1)?,
                        line_number: row.get(2)?,
                        content: row.get(3)?,
                        color: row.get(4)?,
                        minimized: minimized != 0,
                        width: row.get(6)?,
                        pinned: pinned != 0,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(notes)
        })
    }

    /// Insert or replace a sticky note.
    pub fn save_note(&self, note: &StickyNote) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO sticky_notes
                    (id, tab_id, line_number, content, color, minimized, width, pinned, created_at, updated_at)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    note.id,
                    note.tab_id,
                    note.line_number,
                    note.content,
                    note.color,
                    note.minimized as i32,
                    note.width,
                    note.pinned as i32,
                    note.created_at,
                    note.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Delete a single note by id (scoped to tab_id for safety).
    pub fn delete_note(&self, note_id: &str, tab_id: &str) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "DELETE FROM sticky_notes WHERE id = ?1 AND tab_id = ?2",
                params![note_id, tab_id],
            )?;
            Ok(())
        })
    }

    /// Move a note to a different line number.
    pub fn move_note(&self, note_id: &str, tab_id: &str, new_line: i32) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE sticky_notes
                 SET line_number = ?1, updated_at = datetime('now')
                 WHERE id = ?2 AND tab_id = ?3",
                params![new_line, note_id, tab_id],
            )?;
            Ok(())
        })
    }

    /// Delete all notes for a tab.
    pub fn delete_tab_notes(&self, tab_id: &str) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "DELETE FROM sticky_notes WHERE tab_id = ?1",
                params![tab_id],
            )?;
            Ok(())
        })
    }
}
