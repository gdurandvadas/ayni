//! Product-semantic threshold classification shared by core signal evaluators.

use crate::signal::Level;

/// Classifies a measured value against inclusive maximum thresholds.
///
/// Values equal to a threshold are offenders, and `fail` takes precedence over
/// `warn`. Minimum threshold classification is intentionally separate so it
/// can be added without changing this maximum-budget contract.
#[must_use]
pub fn classify_maximum<T: PartialOrd>(value: T, warn: T, fail: T) -> Option<Level> {
    if value >= fail {
        Some(Level::Fail)
    } else if value >= warn {
        Some(Level::Warn)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::classify_maximum;
    use crate::signal::Level;

    #[test]
    fn maximum_thresholds_are_inclusive_and_fail_takes_precedence() {
        assert_eq!(classify_maximum(9, 10, 20), None);
        assert_eq!(classify_maximum(10, 10, 20), Some(Level::Warn));
        assert_eq!(classify_maximum(19, 10, 20), Some(Level::Warn));
        assert_eq!(classify_maximum(20, 10, 20), Some(Level::Fail));
        assert_eq!(classify_maximum(21, 10, 20), Some(Level::Fail));
    }
}
