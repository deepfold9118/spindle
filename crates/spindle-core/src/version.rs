use std::cmp::Ordering;

/// Compare dotted version strings numerically ("26.1.10" > "26.1.7").
/// Non-numeric suffixes inside a component ("7a") compare by their leading digits,
/// then lexically by the remainder.
pub fn compare(a: &str, b: &str) -> Ordering {
    let key = |s: &str| -> Vec<(u64, String)> {
        s.split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
                let rest = part[digits.len()..].to_string();
                (digits.parse().unwrap_or(0), rest)
            })
            .collect()
    };
    key(a).cmp(&key(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_not_lexical() {
        assert_eq!(compare("26.1.10", "26.1.7"), Ordering::Greater);
        assert_eq!(compare("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(compare("5.3.0", "5.3.0"), Ordering::Equal);
    }
}
