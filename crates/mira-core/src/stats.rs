//! Small helpers for showing progress and usage numbers.

/// `part` as a whole-number percentage of `total`.
pub fn percent(part: u32, total: u32) -> u32 {
    part * 100 / total
}

/// The average of `values`.
pub fn average(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// The `n` largest values, biggest first.
pub fn top(values: &[u64], n: usize) -> Vec<u64> {
    let mut v = values.to_vec();
    v.sort();
    v[v.len() - n..].to_vec()
}
