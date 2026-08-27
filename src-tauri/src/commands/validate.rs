pub fn validate_name(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("Name cannot be empty".into());
    }
    if t.len() > 255 {
        return Err("Name too long (max 255)".into());
    }
    if t.chars().any(|c| c.is_control()) {
        return Err("Name contains control characters".into());
    }
    Ok(t.to_owned())
}

pub fn validate_port(p: Option<i32>) -> Result<Option<i32>, String> {
    if let Some(v) = p {
        if !(1..=65535).contains(&v) {
            return Err("Port out of range (1-65535)".into());
        }
    }
    Ok(p)
}

pub fn validate_retention(days: i32) -> Result<i32, String> {
    if !(1..=3650).contains(&days) {
        return Err("Retention must be 1-3650 days".into());
    }
    Ok(days)
}

pub fn validate_max_result_rows(n: i32) -> Result<i32, String> {
    if !(1..=100_000).contains(&n) {
        return Err("max_result_rows must be 1-100000".into());
    }
    Ok(n)
}

pub fn validate_preview_limit(n: i32) -> Result<i32, String> {
    if !(1..=1_000).contains(&n) {
        return Err("reference_preview_row_limit must be 1-1000".into());
    }
    Ok(n)
}
