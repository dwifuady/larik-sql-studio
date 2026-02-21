// JSON Export Engine (T035)
// Fast JSON export with streaming support for large datasets

use super::{ExportError, ExportOptions, ExportProgress};
use crate::db::{CellValue, ColumnInfo};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// JSON Exporter for query results
pub struct JsonExporter {
    options: ExportOptions,
}

impl JsonExporter {
    pub fn new(options: ExportOptions) -> Self {
        Self { options }
    }

    pub fn with_default_options() -> Self {
        Self {
            options: ExportOptions {
                pretty_print: true,
                ..Default::default()
            },
        }
    }

    /// Export query results to a JSON string (for smaller datasets)
    pub fn export_to_string(
        &self,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
    ) -> Result<String, ExportError> {
        let json_value = self.build_json_value(columns, rows)?;

        if self.options.pretty_print {
            serde_json::to_string_pretty(&json_value)
                .map_err(|e| ExportError::SerializationError(e.to_string()))
        } else {
            serde_json::to_string(&json_value)
                .map_err(|e| ExportError::SerializationError(e.to_string()))
        }
    }

    /// Export query results to a file with progress reporting
    pub fn export_to_file(
        &self,
        path: &Path,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<ExportProgress, ExportError> {
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);

        self.write_json_streaming(&mut writer, columns, rows, cancel_flag)?;

        let bytes_written = writer.get_ref().metadata()?.len() as usize;
        writer.flush()?;

        Ok(ExportProgress::completed(rows.len(), bytes_written))
    }

    /// Export to file with async progress events
    pub async fn export_to_file_with_progress(
        &self,
        path: &Path,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
        cancel_flag: Arc<AtomicBool>,
        progress_tx: mpsc::Sender<ExportProgress>,
    ) -> Result<ExportProgress, ExportError> {
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);

        let total_rows = rows.len().min(self.options.max_rows.unwrap_or(usize::MAX));
        let report_interval = (total_rows / 100).max(1000).min(10000);

        let indent = if self.options.pretty_print { "  " } else { "" };
        let newline = if self.options.pretty_print { "\n" } else { "" };

        // Start array
        write!(writer, "[{}", newline)?;

        let mut bytes_written = 1usize;

        for (i, row) in rows.iter().take(total_rows).enumerate() {
            // Check cancellation
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }

            // Build row object
            let obj = self.row_to_json_object(columns, row);
            let json_str = if self.options.pretty_print {
                serde_json::to_string_pretty(&obj)
            } else {
                serde_json::to_string(&obj)
            }
            .map_err(|e| ExportError::SerializationError(e.to_string()))?;

            // Write comma if not first row
            if i > 0 {
                write!(writer, ",{}", newline)?;
                bytes_written += 1 + newline.len();
            }

            // Write indented row
            if self.options.pretty_print {
                for line in json_str.lines() {
                    write!(writer, "{}{}", indent, line)?;
                    bytes_written += indent.len() + line.len();
                    if !json_str.ends_with(line) {
                        writeln!(writer)?;
                        bytes_written += 1;
                    }
                }
            } else {
                write!(writer, "{}", json_str)?;
                bytes_written += json_str.len();
            }

            // Report progress
            if i % report_interval == 0 && i > 0 {
                let progress = ExportProgress {
                    rows_exported: i,
                    total_rows,
                    bytes_written,
                    is_complete: false,
                    error: None,
                };
                let _ = progress_tx.send(progress).await;
            }
        }

        // End array
        write!(writer, "{}]", newline)?;

        writer.flush()?;
        let final_bytes = writer.get_ref().metadata()?.len() as usize;

        let final_progress = ExportProgress::completed(total_rows, final_bytes);
        let _ = progress_tx.send(final_progress.clone()).await;

        Ok(final_progress)
    }

    fn build_json_value(
        &self,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
    ) -> Result<Value, ExportError> {
        let max_rows = self.options.max_rows.unwrap_or(usize::MAX);
        let array: Vec<Value> = rows
            .iter()
            .take(max_rows)
            .map(|row| self.row_to_json_object(columns, row))
            .collect();

        Ok(Value::Array(array))
    }

    fn row_to_json_object(&self, columns: &[ColumnInfo], row: &[CellValue]) -> Value {
        let mut obj = Map::new();

        for (col, value) in columns.iter().zip(row.iter()) {
            let json_value = self.cell_to_json_value(value);
            obj.insert(col.name.clone(), json_value);
        }

        Value::Object(obj)
    }

    fn cell_to_json_value(&self, value: &CellValue) -> Value {
        match value {
            CellValue::Null => Value::Null,
            CellValue::Bool(b) => Value::Bool(*b),
            CellValue::Int(i) => json!(*i),
            CellValue::Float(f) => {
                // Handle special float values
                if f.is_nan() {
                    Value::String("NaN".to_string())
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        Value::String("Infinity".to_string())
                    } else {
                        Value::String("-Infinity".to_string())
                    }
                } else {
                    json!(*f)
                }
            }
            CellValue::String(s) => Value::String(s.clone()),
            CellValue::DateTime(dt) => Value::String(dt.clone()),
            CellValue::Binary(bytes) => {
                // Convert binary to base64 for JSON
                let encoded = base64_encode(bytes);
                json!({
                    "_type": "binary",
                    "encoding": "base64",
                    "data": encoded
                })
            }
        }
    }

    fn write_json_streaming<W: Write>(
        &self,
        writer: &mut W,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<(), ExportError> {
        let max_rows = self.options.max_rows.unwrap_or(usize::MAX);
        let indent = if self.options.pretty_print { "  " } else { "" };
        let newline = if self.options.pretty_print { "\n" } else { "" };

        // Start array
        write!(writer, "[{}", newline)?;

        for (i, row) in rows.iter().take(max_rows).enumerate() {
            // Check cancellation
            if let Some(ref flag) = cancel_flag {
                if i % 1000 == 0 && flag.load(Ordering::Relaxed) {
                    return Err(ExportError::Cancelled);
                }
            }

            // Build row object
            let obj = self.row_to_json_object(columns, row);
            let json_str = if self.options.pretty_print {
                serde_json::to_string_pretty(&obj)
            } else {
                serde_json::to_string(&obj)
            }
            .map_err(|e| ExportError::SerializationError(e.to_string()))?;

            // Write comma if not first row
            if i > 0 {
                write!(writer, ",{}", newline)?;
            }

            // Write indented row
            if self.options.pretty_print {
                for (j, line) in json_str.lines().enumerate() {
                    if j > 0 {
                        writeln!(writer)?;
                    }
                    write!(writer, "{}{}", indent, line)?;
                }
            } else {
                write!(writer, "{}", json_str)?;
            }
        }

        // End array
        writeln!(writer, "{}]", newline)?;

        Ok(())
    }
}

/// Simple base64 encoding (avoiding additional dependencies)
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::new();
    let chunks = data.chunks(3);

    for chunk in chunks {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_columns() -> Vec<ColumnInfo> {
        vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "Int4".to_string(),
                nullable: false,
            },
            ColumnInfo {
                name: "name".to_string(),
                data_type: "NVarchar".to_string(),
                nullable: true,
            },
            ColumnInfo {
                name: "active".to_string(),
                data_type: "Bit".to_string(),
                nullable: true,
            },
        ]
    }

    fn sample_rows() -> Vec<Vec<CellValue>> {
        vec![
            vec![
                CellValue::Int(1),
                CellValue::String("Alice".to_string()),
                CellValue::Bool(true),
            ],
            vec![
                CellValue::Int(2),
                CellValue::String("Bob".to_string()),
                CellValue::Null,
            ],
        ]
    }

    #[test]
    fn test_json_export_basic() {
        let exporter = JsonExporter::new(ExportOptions {
            pretty_print: false,
            ..Default::default()
        });
        let result = exporter.export_to_string(&sample_columns(), &sample_rows());

        assert!(result.is_ok());
        let json = result.unwrap();

        // Parse and verify
        let parsed: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);

        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(parsed[0]["name"], "Alice");
        assert_eq!(parsed[0]["active"], true);

        assert_eq!(parsed[1]["id"], 2);
        assert_eq!(parsed[1]["name"], "Bob");
        assert!(parsed[1]["active"].is_null());
    }

    #[test]
    fn test_json_export_pretty_print() {
        let exporter = JsonExporter::with_default_options();
        let result = exporter.export_to_string(&sample_columns(), &sample_rows());

        assert!(result.is_ok());
        let json = result.unwrap();

        // Should have indentation
        assert!(json.contains("  "));
        assert!(json.contains("\n"));
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b"Hi"), "SGk=");
        assert_eq!(base64_encode(b"A"), "QQ==");
    }
}
