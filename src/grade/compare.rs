//! Judging a program's stdout against the expected output, on the host.
use crate::scenario::{Compare, Tolerance};

/// Whether `actual` matches `expected` under `rule`.
pub(super) fn matches(rule: Compare, actual: &[u8], expected: &[u8]) -> bool {
    match rule {
        Compare::Exact => actual == expected,
        Compare::Whitespace => tokens(actual).eq(tokens(expected)),
        Compare::Numeric(tolerance) => {
            let (mut actual, mut expected) = (tokens(actual), tokens(expected));
            loop {
                match (actual.next(), expected.next()) {
                    (None, None) => return true,
                    (Some(a), Some(e)) if within(tolerance, a, e) => {}
                    _ => return false,
                }
            }
        }
    }
}

fn tokens(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(u8::is_ascii_whitespace)
        .filter(|token| !token.is_empty())
}

/// Equal tokens match. Otherwise both must be finite numbers within the tolerance.
fn within(tolerance: Tolerance, actual: &[u8], expected: &[u8]) -> bool {
    if actual == expected {
        return true;
    }
    match (number(actual), number(expected)) {
        (Some(a), Some(e)) => {
            let diff = (a - e).abs();
            diff <= tolerance.abs || diff <= tolerance.rel * e.abs()
        }
        _ => false,
    }
}

fn number(token: &[u8]) -> Option<f64> {
    std::str::from_utf8(token)
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NUMERIC: Compare = Compare::Numeric(Tolerance {
        abs: 1e-6,
        rel: 1e-3,
    });

    #[test]
    fn exact_compares_bytes() {
        assert!(matches(Compare::Exact, b"a 1\n", b"a 1\n"));
        assert!(!matches(Compare::Exact, b"a 1", b"a 1\n"));
        assert!(!matches(Compare::Exact, b"a  1\n", b"a 1\n"));
    }

    #[test]
    fn whitespace_compares_tokens_however_spaced() {
        assert!(matches(Compare::Whitespace, b" a\t1\r\n\n", b"a 1\n"));
        assert!(matches(Compare::Whitespace, b"", b"\n \n"));
        assert!(!matches(Compare::Whitespace, b"a 1 2\n", b"a 1\n"));
        assert!(!matches(Compare::Whitespace, b"a1\n", b"a 1\n"));
        assert!(!matches(Compare::Whitespace, b"1.0\n", b"1\n"));
    }

    #[test]
    fn numeric_accepts_numbers_within_either_tolerance() {
        assert!(matches(NUMERIC, b"x 1.0000005\n", b"x 1\n"));
        assert!(matches(NUMERIC, b"1000.9", b"1000"));
        assert!(matches(NUMERIC, b"1e3", b"1000"));
        assert!(!matches(NUMERIC, b"1001.5", b"1000"));
        assert!(!matches(NUMERIC, b"0.001", b"0"));
    }

    #[test]
    fn numeric_needs_the_same_words_and_count() {
        assert!(!matches(NUMERIC, b"y 1", b"x 1"));
        assert!(!matches(NUMERIC, b"1 2", b"1"));
        assert!(!matches(NUMERIC, b"1", b"1 2"));
        assert!(matches(NUMERIC, b"nan inf", b"nan inf"));
        assert!(!matches(NUMERIC, b"inf", b"1e308"));
    }
}
