//! eSCL ScanJobs identifier parsing.

/// Extract an eSCL job id from HTTP headers or response body.
///
/// Handles absolute Location URLs like `http://host:80/eSCL/ScanJobs/uuid`
/// via `ScanJobs/([^/]+)` — never `split(':').nth(1)` (that yields `"http"`).
pub fn parse_job_id(headers: &str, body: &[u8]) -> Option<String> {
    if let Some(id) = headers.lines().find_map(location_job_id) {
        return Some(id);
    }
    let text = String::from_utf8_lossy(body);
    scan_job_id(&text)
}

fn location_job_id(line: &str) -> Option<String> {
    let (name, value) = line.split_once(':')?;
    if !name.eq_ignore_ascii_case("location") {
        return None;
    }
    scan_job_id(value).or_else(|| relative_location_job_id(value))
}

fn relative_location_job_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return None;
    }
    value
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
}

fn scan_job_id(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    lower
        .match_indices("scanjobs/")
        .filter_map(|(start, _)| job_id_after(&value[start + "scanjobs/".len()..]))
        .next()
}

fn job_id_after(value: &str) -> Option<String> {
    let id = value
        .split(|character: char| {
            matches!(
                character,
                '/' | '?' | '#' | '<' | '>' | '"' | '\'' | '&' | ';' | ','
            ) || character.is_whitespace()
        })
        .next()?;
    (!id.is_empty() && !matches!(id, "http" | "https")).then(|| id.to_string())
}
