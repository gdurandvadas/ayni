use std::cmp::Ordering;
use std::path::Path;

use serde_json::Value;

pub(super) fn sort_offenders_desc_numeric(offenders: &mut [Value]) {
    offenders.sort_by(|left, right| {
        let left_value = left
            .get("value")
            .and_then(Value::as_f64)
            .unwrap_or(f64::MIN);
        let right_value = right
            .get("value")
            .and_then(Value::as_f64)
            .unwrap_or(f64::MIN);
        right_value
            .partial_cmp(&left_value)
            .unwrap_or(Ordering::Equal)
    });
}

pub(super) fn to_relative_posix(repo_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(repo_root).map_err(|err| {
        format!(
            "path {} is outside repo {}: {err}",
            path.display(),
            repo_root.display()
        )
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

pub(super) fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
