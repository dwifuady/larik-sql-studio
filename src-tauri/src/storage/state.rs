// App state and settings management
// Handles persistent key-value storage in the app_state table

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::database::{DatabaseManager, StorageResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoArchiveSettings {
    pub enabled: bool,
    pub days_inactive: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub validation_enabled: bool,
    pub last_space_id: Option<String>,
    pub last_tab_id: Option<String>,
    pub enable_sticky_notes: bool,
    pub max_result_rows: i32,
}

impl DatabaseManager {
    /// Get a setting value by key
    pub fn get_setting(&self, key: &str) -> StorageResult<Option<String>> {
        self.with_connection(|conn| {
            let value: Option<String> = conn
                .query_row(
                    "SELECT value FROM app_state WHERE key = ?",
                    params![key],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(value)
        })
    }

    /// Set a setting value by key
    pub fn set_setting(&self, key: &str, value: &str) -> StorageResult<()> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO app_state (key, value, updated_at)
                 VALUES (?, ?, datetime('now'))",
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// Get auto-archive enabled setting
    pub fn get_auto_archive_enabled(&self) -> StorageResult<bool> {
        let value = self.get_setting("auto_archive_enabled")?;
        Ok(value.as_deref() == Some("true"))
    }

    /// Get auto-archive days setting
    pub fn get_auto_archive_days(&self) -> StorageResult<i32> {
        let value = self.get_setting("auto_archive_days")?;
        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(14))
    }

    /// Get history retention days setting
    pub fn get_history_retention_days(&self) -> StorageResult<i32> {
        let value = self.get_setting("history_retention_days")?;
        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(90))
    }

    /// Get auto-archive settings as a struct
    pub fn get_auto_archive_settings(&self) -> StorageResult<AutoArchiveSettings> {
        Ok(AutoArchiveSettings {
            enabled: self.get_auto_archive_enabled()?,
            days_inactive: self.get_auto_archive_days()?,
        })
    }

    /// Update auto-archive settings
    pub fn update_auto_archive_settings(
        &self,
        enabled: bool,
        days_inactive: i32,
    ) -> StorageResult<()> {
        self.set_setting(
            "auto_archive_enabled",
            if enabled { "true" } else { "false" },
        )?;
        self.set_setting("auto_archive_days", &days_inactive.to_string())?;
        Ok(())
    }

    /// Get validation enabled setting
    pub fn get_validation_enabled(&self) -> StorageResult<bool> {
        let value = self.get_setting("validation_enabled")?;
        // Default to true if not set
        Ok(value.as_deref() != Some("false"))
    }

    /// Set validation enabled setting
    pub fn set_validation_enabled(&self, enabled: bool) -> StorageResult<()> {
        self.set_setting("validation_enabled", if enabled { "true" } else { "false" })
    }

    /// Get sticky notes enabled setting
    pub fn get_enable_sticky_notes(&self) -> StorageResult<bool> {
        let value = self.get_setting("enable_sticky_notes")?;
        // Default to true if not set
        Ok(value.as_deref() != Some("false"))
    }

    /// Set sticky notes enabled setting
    pub fn set_enable_sticky_notes(&self, enabled: bool) -> StorageResult<()> {
        self.set_setting(
            "enable_sticky_notes",
            if enabled { "true" } else { "false" },
        )
    }

    /// Get max result rows setting
    pub fn get_max_result_rows(&self) -> StorageResult<i32> {
        let value = self.get_setting("max_result_rows")?;
        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(5000))
    }

    /// Set max result rows setting
    pub fn set_max_result_rows(&self, max_rows: i32) -> StorageResult<()> {
        self.set_setting("max_result_rows", &max_rows.to_string())
    }

    /// Get last opened space ID
    pub fn get_last_space_id(&self) -> StorageResult<Option<String>> {
        self.get_setting("last_space_id")
    }

    /// Set last opened space ID
    pub fn set_last_space_id(&self, space_id: Option<&str>) -> StorageResult<()> {
        if let Some(id) = space_id {
            self.set_setting("last_space_id", id)
        } else {
            // Remove the setting if None
            self.with_connection(|conn| {
                conn.execute("DELETE FROM app_state WHERE key = 'last_space_id'", [])?;
                Ok(())
            })
        }
    }

    /// Get last opened tab ID
    pub fn get_last_tab_id(&self) -> StorageResult<Option<String>> {
        self.get_setting("last_tab_id")
    }

    /// Set last opened tab ID
    pub fn set_last_tab_id(&self, tab_id: Option<&str>) -> StorageResult<()> {
        if let Some(id) = tab_id {
            self.set_setting("last_tab_id", id)
        } else {
            // Remove the setting if None
            self.with_connection(|conn| {
                conn.execute("DELETE FROM app_state WHERE key = 'last_tab_id'", [])?;
                Ok(())
            })
        }
    }

    /// Get app settings as a struct
    pub fn get_app_settings(&self) -> StorageResult<AppSettings> {
        Ok(AppSettings {
            validation_enabled: self.get_validation_enabled()?,
            last_space_id: self.get_last_space_id()?,
            last_tab_id: self.get_last_tab_id()?,
            enable_sticky_notes: self.get_enable_sticky_notes()?,
            max_result_rows: self.get_max_result_rows()?,
        })
    }

    /// Update app settings
    pub fn update_app_settings(&self, settings: &AppSettings) -> StorageResult<()> {
        self.set_validation_enabled(settings.validation_enabled)?;
        self.set_last_space_id(settings.last_space_id.as_deref())?;
        self.set_last_tab_id(settings.last_tab_id.as_deref())?;
        self.set_enable_sticky_notes(settings.enable_sticky_notes)?;
        self.set_max_result_rows(settings.max_result_rows)?;
        Ok(())
    }

    /// Initialize default settings if they don't exist
    pub fn init_default_settings(&self) -> StorageResult<()> {
        if self.get_setting("auto_archive_enabled")?.is_none() {
            self.set_setting("auto_archive_enabled", "true")?;
        }
        if self.get_setting("auto_archive_days")?.is_none() {
            self.set_setting("auto_archive_days", "14")?;
        }
        if self.get_setting("history_retention_days")?.is_none() {
            self.set_setting("history_retention_days", "90")?;
        }
        if self.get_setting("validation_enabled")?.is_none() {
            self.set_setting("validation_enabled", "true")?;
        }
        if self.get_setting("enable_sticky_notes")?.is_none() {
            self.set_setting("enable_sticky_notes", "true")?;
        }
        if self.get_setting("max_result_rows")?.is_none() {
            self.set_setting("max_result_rows", "5000")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_db() -> DatabaseManager {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("larik_state_test_{}.db", Uuid::new_v4()));
        DatabaseManager::new(db_path).unwrap()
    }

    #[test]
    fn test_get_set_setting() {
        let db = create_test_db();

        // Initially None
        assert_eq!(db.get_setting("test_key").unwrap(), None);

        // Set and get
        db.set_setting("test_key", "test_value").unwrap();
        assert_eq!(
            db.get_setting("test_key").unwrap(),
            Some("test_value".to_string())
        );

        // Update
        db.set_setting("test_key", "new_value").unwrap();
        assert_eq!(
            db.get_setting("test_key").unwrap(),
            Some("new_value".to_string())
        );
    }

    #[test]
    fn test_auto_archive_settings() {
        let db = create_test_db();

        // Default values
        assert_eq!(db.get_auto_archive_enabled().unwrap(), false);
        assert_eq!(db.get_auto_archive_days().unwrap(), 14);

        // Update settings
        db.update_auto_archive_settings(true, 7).unwrap();
        assert_eq!(db.get_auto_archive_enabled().unwrap(), true);
        assert_eq!(db.get_auto_archive_days().unwrap(), 7);

        // Get as struct
        let settings = db.get_auto_archive_settings().unwrap();
        assert_eq!(settings.enabled, true);
        assert_eq!(settings.days_inactive, 7);
    }

    #[test]
    fn test_init_default_settings() {
        let db = create_test_db();

        db.init_default_settings().unwrap();

        assert_eq!(db.get_auto_archive_enabled().unwrap(), true);
        assert_eq!(db.get_auto_archive_days().unwrap(), 14);
        assert_eq!(db.get_history_retention_days().unwrap(), 90);
    }
}
