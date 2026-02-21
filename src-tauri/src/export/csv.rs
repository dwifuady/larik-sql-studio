// CSV Export Engine (T034)
// Fast CSV export with streaming support for large datasets

use super::{ExportError, ExportOptions, ExportProgress};
use crate::db::{CellValue, ColumnInfo};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// CSV Exporter for query results
pub struct CsvExporter {
    options: ExportOptions,
}

impl CsvExporter {
    pub fn new(options: ExportOptions) -> Self {
        Self { options }
    }

    pub fn with_default_options() -> Self {
        Self {
            options: ExportOptions::default(),
        }
    }

    /// Export query results to a CSV string (for smaller datasets)
    pub fn export_to_string(
        &self,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
    ) -> Result<String, ExportError> {
        let mut output = Vec::new();
        self.write_csv(&mut output, columns, rows, None)?;
        String::from_utf8(output).map_err(|e| ExportError::SerializationError(e.to_string()))
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
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file); // 64KB buffer

        self.write_csv(&mut writer, columns, rows, cancel_flag)?;

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

        let delimiter = self.get_delimiter();
        let quote = self.get_quote_char();
        let total_rows = rows.len().min(self.options.max_rows.unwrap_or(usize::MAX));

        // Write header
        if self.options.include_headers {
            self.write_header(&mut writer, columns, delimiter, quote)?;
        }

        // Write rows with progress reporting
        let report_interval = (total_rows / 100).max(1000).min(10000); // Report every 1-10%
        let mut bytes_written = 0usize;

        for (i, row) in rows.iter().take(total_rows).enumerate() {
            // Check cancellation
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }

            bytes_written += self.write_row(&mut writer, row, delimiter, quote)?;

            // Report progress periodically
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

        writer.flush()?;
        let final_bytes = writer.get_ref().metadata()?.len() as usize;

        let final_progress = ExportProgress::completed(total_rows, final_bytes);
        let _ = progress_tx.send(final_progress.clone()).await;

        Ok(final_progress)
    }

    fn write_csv<W: Write>(
        &self,
        writer: &mut W,
        columns: &[ColumnInfo],
        rows: &[Vec<CellValue>],
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<(), ExportError> {
        let delimiter = self.get_delimiter();
        let quote = self.get_quote_char();
        let max_rows = self.options.max_rows.unwrap_or(usize::MAX);

        // Write header
        if self.options.include_headers {
            self.write_header(writer, columns, delimiter, quote)?;
        }

        // Write rows
        for (i, row) in rows.iter().take(max_rows).enumerate() {
            // Check cancellation periodically
            if let Some(ref flag) = cancel_flag {
                if i % 1000 == 0 && flag.load(Ordering::Relaxed) {
                    return Err(ExportError::Cancelled);
                }
            }

            self.write_row(writer, row, delimiter, quote)?;
        }

        Ok(())
    }

    fn write_header<W: Write>(
        &self,
        writer: &mut W,
        columns: &[ColumnInfo],
        delimiter: char,
        quote: char,
    ) -> Result<(), ExportError> {
        let header: Vec<String> = columns
            .iter()
            .map(|col| self.escape_csv_field(&col.name, delimiter, quote))
            .collect();

        writeln!(writer, "{}", header.join(&delimiter.to_string()))?;
        Ok(())
    }

    fn write_row<W: Write>(
        &self,
        writer: &mut W,
        row: &[CellValue],
        delimiter: char,
        quote: char,
    ) -> Result<usize, ExportError> {
        let fields: Vec<String> = row
            .iter()
            .map(|cell| self.format_cell_value(cell, delimiter, quote))
            .collect();

        let line = format!("{}\n", fields.join(&delimiter.to_string()));
        let bytes = line.len();
        writer.write_all(line.as_bytes())?;
        Ok(bytes)
    }

    fn format_cell_value(&self, value: &CellValue, delimiter: char, quote: char) -> String {
        match value {
            CellValue::Null => {
                if self.options.null_as_string {
                    "NULL".to_string()
                } else {
                    String::new()
                }
            }
            CellValue::Bool(b) => b.to_string(),
            CellValue::Int(i) => i.to_string(),
            CellValue::Float(f) => {
                // Format floats without unnecessary precision
                if f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    f.to_string()
                }
            }
            CellValue::String(s) => self.escape_csv_field(s, delimiter, quote),
            CellValue::DateTime(dt) => self.escape_csv_field(dt, delimiter, quote),
            CellValue::Binary(bytes) => {
                // Convert binary to hex representation
                let hex = bytes
                    .iter()
                    .take(100) // Limit to first 100 bytes for readability
                    .map(|b| format!("{:02X}", b))
                    .collect::<String>();
                if bytes.len() > 100 {
                    format!("0x{}...", hex)
                } else {
                    format!("0x{}", hex)
                }
            }
        }
    }

    fn escape_csv_field(&self, value: &str, delimiter: char, quote: char) -> String {
        // Check if field needs quoting
        let needs_quoting = value.contains(delimiter)
            || value.contains(quote)
            || value.contains('\n')
            || value.contains('\r');

        if needs_quoting {
            // Escape quotes by doubling them
            let escaped = value.replace(quote, &format!("{}{}", quote, quote));
            format!("{}{}{}", quote, escaped, quote)
        } else {
            value.to_string()
        }
    }

    fn get_delimiter(&self) -> char {
        self.options
            .delimiter
            .as_ref()
            .and_then(|s| s.chars().next())
            .unwrap_or(',')
    }

    fn get_quote_char(&self) -> char {
        self.options
            .quote_char
            .as_ref()
            .and_then(|s| s.chars().next())
            .unwrap_or('"')
    }
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
                name: "value".to_string(),
                data_type: "Float8".to_string(),
                nullable: true,
            },
        ]
    }

    fn sample_rows() -> Vec<Vec<CellValue>> {
        vec![
            vec![
                CellValue::Int(1),
                CellValue::String("Alice".to_string()),
                CellValue::Float(100.5),
            ],
            vec![
                CellValue::Int(2),
                CellValue::String("Bob, Jr.".to_string()), // Contains comma
                CellValue::Null,
            ],
            vec![
                CellValue::Int(3),
                CellValue::String("Charlie \"The Great\"".to_string()), // Contains quotes
                CellValue::Float(200.0),
            ],
        ]
    }

    #[test]
    fn test_csv_export_basic() {
        let exporter = CsvExporter::with_default_options();
        let result = exporter.export_to_string(&sample_columns(), &sample_rows());

        assert!(result.is_ok());
        let csv = result.unwrap();

        // Check header
        assert!(csv.starts_with("id,name,value\n"));

        // Check data
        assert!(csv.contains("1,Alice,100.5"));
        assert!(csv.contains("\"Bob, Jr.\"")); // Comma should be quoted
        assert!(csv.contains("\"Charlie \"\"The Great\"\"\"")); // Quotes should be escaped
        assert!(csv.contains("NULL")); // NULL value
    }

    #[test]
    fn test_csv_export_no_headers() {
        let options = ExportOptions {
            include_headers: false,
            ..Default::default()
        };
        let exporter = CsvExporter::new(options);
        let result = exporter.export_to_string(&sample_columns(), &sample_rows());

        assert!(result.is_ok());
        let csv = result.unwrap();

        // Should not start with header
        assert!(csv.starts_with("1,Alice,100.5"));
    }

    #[test]
    fn test_csv_export_custom_delimiter() {
        let options = ExportOptions {
            delimiter: Some(";".to_string()),
            ..Default::default()
        };
        let exporter = CsvExporter::new(options);
        let result = exporter.export_to_string(&sample_columns(), &sample_rows());

        assert!(result.is_ok());
        let csv = result.unwrap();

        // Should use semicolon as delimiter
        assert!(csv.contains("id;name;value"));
        assert!(csv.contains("1;Alice;100.5"));
    }
}
