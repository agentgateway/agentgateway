use std::sync::LazyLock;

use chrono::TimeZone;

use crate::{ExecutionError, ResolveResult, Value};

// Timestamp values are limited to the range of values which can be serialized as a string:
// `["0001-01-01T00:00:00Z", "9999-12-31T23:59:59.999999999Z"]`. Since the max is a smaller
// and the min is a larger timestamp than what is possible to represent with [`DateTime`],
// we need to perform our own spec-compliant overflow checks.
//
// https://github.com/google/cel-spec/blob/master/doc/langdef.md#overflow

static MAX_TIMESTAMP: LazyLock<chrono::DateTime<chrono::FixedOffset>> = LazyLock::new(|| {
    let naive = chrono::NaiveDate::from_ymd_opt(9999, 12, 31)
        .unwrap()
        .and_hms_nano_opt(23, 59, 59, 999_999_999)
        .unwrap();
    chrono::FixedOffset::east_opt(0)
        .unwrap()
        .from_utc_datetime(&naive)
});

static MIN_TIMESTAMP: LazyLock<chrono::DateTime<chrono::FixedOffset>> = LazyLock::new(|| {
    let naive = chrono::NaiveDate::from_ymd_opt(1, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    chrono::FixedOffset::east_opt(0)
        .unwrap()
        .from_utc_datetime(&naive)
});

pub(crate) enum TsOp {
    Add,
    Sub,
}

impl TsOp {
    fn str(&self) -> &'static str {
        match self {
            TsOp::Add => "add",
            TsOp::Sub => "sub",
        }
    }
}

/// Performs a checked arithmetic operation [`TsOp`] on a timestamp and a duration and ensures that
/// the resulting timestamp does not overflow the data type internal limits, as well as the timestamp
/// limits defined in the cel-spec. See [`MAX_TIMESTAMP`] and [`MIN_TIMESTAMP`] for more details.
pub(crate) fn checked_op<'a>(
    op: TsOp,
    lhs: &chrono::DateTime<chrono::FixedOffset>,
    rhs: &chrono::Duration,
) -> ResolveResult<'a> {
    // Add lhs and rhs together, checking for data type overflow
    let result = match op {
        TsOp::Add => lhs.checked_add_signed(*rhs),
        TsOp::Sub => lhs.checked_sub_signed(*rhs),
    }
    .ok_or(ExecutionError::Overflow(
        op.str(),
        (*lhs).into(),
        (*rhs).into(),
    ))?;

    // Check for cel-spec limits
    if result > *MAX_TIMESTAMP || result < *MIN_TIMESTAMP {
        Err(ExecutionError::Overflow(
            op.str(),
            (*lhs).into(),
            (*rhs).into(),
        ))
    } else {
        Value::Timestamp(result).into()
    }
}
