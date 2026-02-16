// Export Module (T034, T035)
// Handles CSV and JSON export functionality with streaming support for large datasets

pub mod csv;
pub mod json;

pub use csv::CsvExporter;
pub use json::JsonExporter;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Export format options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
}

/// Export options for customizing output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Include column headers (CSV) or use them as JSON keys
    pub include_headers: bool,
    /// Pretty print JSON output
    pub pretty_print: bool,
    /// Delimiter for CSV (default: comma)
    pub delimiter: Option<String>,
    /// Quote character for CSV (default: double quote)
    pub quote_char: Option<String>,
    /// Include NULL values as "NULL" string or empty
    pub null_as_string: bool,
    /// Maximum rows to export (None = all rows)
    pub max_rows: Option<usize>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_headers: true,
            pretty_print: false,
            delimiter: Some(",".to_string()),
            quote_char: Some("\"".to_string()),
            null_as_string: true,
            max_rows: None,
        }
    }
}

/// Export progress information for UI updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportProgress {
    pub rows_exported: usize,
    pub total_rows: usize,
    pub bytes_written: usize,
    pub is_complete: bool,
    pub error: Option<String>,
}

impl ExportProgress {
    pub fn new(total_rows: usize) -> Self {
        Self {
            rows_exported: 0,
            total_rows,
            bytes_written: 0,
            is_complete: false,
            error: None,
        }
    }

    pub fn completed(rows_exported: usize, bytes_written: usize) -> Self {
        Self {
            rows_exported,
            total_rows: rows_exported,
            bytes_written,
            is_complete: true,
            error: None,
        }
    }

    pub fn with_error(error: String) -> Self {
        Self {
            rows_exported: 0,
            total_rows: 0,
            bytes_written: 0,
            is_complete: true,
            error: Some(error),
        }
    }
}

/// Export errors
#[derive(Error, Debug)]
pub enum ExportError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Export cancelled")]
    Cancelled,

    #[error("Invalid export options: {0}")]
    InvalidOptions(String),

    #[error("No data to export")]
    NoData,
}
