//! Fuzzy matching for the palette and the file picker.

/// Scores `candidate` for a lowercase `query`, or `None` if the query's
/// characters don't all appear in it, in order. Higher is better: matches at
/// word starts, in a row, or in the file name count for more, and shorter
/// candidates win ties.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let query: Vec<char> = query.chars().collect();
    let name = chars.iter().rposition(|c| *c == '/').map_or(0, |i| i + 1);
    // Matching only within the file name usually scores best, but a query
    // can span folders, so try both.
    let best = [name, 0]
        .into_iter()
        .filter_map(|from| greedy(&query, &chars, &lower, from, name))
        .max()?;
    Some(best - chars.len() as i64 / 8)
}

fn greedy(query: &[char], chars: &[char], lower: &[char], from: usize, name: usize) -> Option<i64> {
    let mut score = 0;
    let mut prev: Option<usize> = None;
    let mut next = from;
    for &q in query {
        let at = (next..lower.len()).find(|&j| lower[j] == q)?;
        score += 1;
        if prev.is_some_and(|p| p + 1 == at) {
            score += 5;
        }
        let word_start = at == 0
            || matches!(chars[at - 1], '/' | '_' | '-' | '.' | ' ' | ':')
            || (chars[at].is_uppercase() && chars[at - 1].is_lowercase());
        if word_start {
            score += 8;
        }
        if at >= name {
            score += 2;
        }
        prev = Some(at);
        next = at + 1;
    }
    Some(score)
}
