//! Product-semantic threshold classification shared by core signal evaluators.

use crate::{policy::ThresholdFloat, signal::Level};

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

/// Classifies a measured value against exclusive minimum thresholds.
///
/// Values equal to a threshold meet that threshold. `fail` takes precedence so
/// a value below both configured minimums is a failure.
#[must_use]
pub fn classify_minimum<T: PartialOrd>(value: T, warn: T, fail: T) -> Option<Level> {
    if value < fail {
        Some(Level::Fail)
    } else if value < warn {
        Some(Level::Warn)
    } else {
        None
    }
}

/// The outcome of evaluating one optionally configured percentage metric.
///
/// A configured metric requires finite evidence. Missing and non-finite values
/// remain distinct so adapters can report actionable setup failures rather than
/// fabricating a zero-percent measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfiguredMetricEvaluation {
    /// No threshold is configured, so the metric is not evaluated.
    Unconfigured,
    /// A finite measurement with its minimum-threshold classification.
    Present { value: f64, level: Option<Level> },
    /// A configured metric was absent from the collected evidence.
    Missing,
    /// A configured metric was present but was not finite.
    Unparseable,
}

/// Evaluates one configured minimum-threshold metric without inventing evidence.
#[must_use]
pub fn evaluate_configured_metric(
    value: Option<f64>,
    threshold: Option<ThresholdFloat>,
) -> ConfiguredMetricEvaluation {
    let Some(threshold) = threshold else {
        return ConfiguredMetricEvaluation::Unconfigured;
    };
    let Some(value) = value else {
        return ConfiguredMetricEvaluation::Missing;
    };
    if !value.is_finite() {
        return ConfiguredMetricEvaluation::Unparseable;
    }
    ConfiguredMetricEvaluation::Present {
        value,
        level: classify_minimum(value, threshold.warn, threshold.fail),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfiguredMetricEvaluation, classify_maximum, evaluate_configured_metric};
    use crate::{ThresholdFloat, signal::Level};

    #[test]
    fn maximum_thresholds_are_inclusive_and_fail_takes_precedence() {
        assert_eq!(classify_maximum(9, 10, 20), None);
        assert_eq!(classify_maximum(10, 10, 20), Some(Level::Warn));
        assert_eq!(classify_maximum(19, 10, 20), Some(Level::Warn));
        assert_eq!(classify_maximum(20, 10, 20), Some(Level::Fail));
        assert_eq!(classify_maximum(21, 10, 20), Some(Level::Fail));
    }

    #[test]
    fn minimum_boundaries_are_exclusive_and_fail_takes_precedence() {
        let threshold = Some(ThresholdFloat {
            warn: 80.0,
            fail: 70.0,
        });
        assert_eq!(
            evaluate_configured_metric(Some(69.0), threshold),
            ConfiguredMetricEvaluation::Present {
                value: 69.0,
                level: Some(Level::Fail)
            }
        );
        assert_eq!(
            evaluate_configured_metric(Some(70.0), threshold),
            ConfiguredMetricEvaluation::Present {
                value: 70.0,
                level: Some(Level::Warn)
            }
        );
        assert_eq!(
            evaluate_configured_metric(Some(80.0), threshold),
            ConfiguredMetricEvaluation::Present {
                value: 80.0,
                level: None
            }
        );
    }

    #[test]
    fn configured_metrics_preserve_absent_and_non_finite_evidence() {
        let threshold = Some(ThresholdFloat {
            warn: 80.0,
            fail: 70.0,
        });
        assert_eq!(
            evaluate_configured_metric(None, threshold),
            ConfiguredMetricEvaluation::Missing
        );
        assert_eq!(
            evaluate_configured_metric(Some(f64::NAN), threshold),
            ConfiguredMetricEvaluation::Unparseable
        );
        assert_eq!(
            evaluate_configured_metric(Some(f64::INFINITY), threshold),
            ConfiguredMetricEvaluation::Unparseable
        );
        assert_eq!(
            evaluate_configured_metric(None, None),
            ConfiguredMetricEvaluation::Unconfigured
        );
    }
}
