//! Fuzzy scorer ported from junegunn/fzf `src/algo/algo.go`
//! (`FuzzyMatchV2`, a modified Smith-Waterman). fzf is MIT licensed; this is
//! an independent Rust implementation of the documented scoring criteria:
//!
//! - word-boundary / camelCase / delimiter bonuses on matched characters
//! - first pattern character bonus multiplied (typed intent)
//! - gap penalties (start + extension) so scattered hits lose to tight runs
//! - consecutive-chunk bonus so runs keep winning over gapped matches
//!
//! Returns the highest-scoring alignment (score, character positions).

const SCORE_MATCH: i32 = 16;
const SCORE_GAP_START: i32 = -3;
const SCORE_GAP_EXT: i32 = -1;

/// Matches at word starts; cancelled once a gap grows past ~8 chars.
const BONUS_BOUNDARY: i32 = SCORE_MATCH / 2;
const BONUS_NONWORD: i32 = SCORE_MATCH / 2;
const BONUS_CAMEL: i32 = BONUS_BOUNDARY + SCORE_GAP_EXT;
const BONUS_CONSECUTIVE: i32 = -(SCORE_GAP_START + SCORE_GAP_EXT);
const BONUS_FIRST_MULT: i32 = 2;
const BONUS_BOUNDARY_WHITE: i32 = BONUS_BOUNDARY + 2;
const BONUS_BOUNDARY_DELIM: i32 = BONUS_BOUNDARY + 1;

/// fzf delimiter set for the default bonus scheme.
const DELIMITERS: &str = "/,:;|";
const WHITE: &[char] = &[' ', '\t', '\n', '\u{b}', '\u{c}', '\r', '\u{85}', '\u{a0}'];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    White = 0,
    NonWord = 1,
    Delimiter = 2,
    Lower = 3,
    Upper = 4,
    Letter = 5,
    Number = 6,
}

fn class_of(c: char) -> Class {
    if c.is_ascii_lowercase() {
        Class::Lower
    } else if c.is_ascii_uppercase() {
        Class::Upper
    } else if c.is_ascii_digit() {
        Class::Number
    } else if WHITE.contains(&c) {
        Class::White
    } else if c.is_ascii() {
        if DELIMITERS.contains(c) {
            Class::Delimiter
        } else {
            Class::NonWord
        }
    } else if c.is_lowercase() {
        Class::Lower
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_numeric() {
        Class::Number
    } else if c.is_alphabetic() {
        Class::Letter
    } else if c.is_whitespace() {
        Class::White
    } else if DELIMITERS.contains(c) {
        Class::Delimiter
    } else {
        Class::NonWord
    }
}

fn bonus_for(prev: Class, class: Class) -> i32 {
    if (class as i32) >= (Class::NonWord as i32) {
        match prev {
            Class::White => return BONUS_BOUNDARY_WHITE,
            Class::Delimiter => return BONUS_BOUNDARY_DELIM,
            Class::NonWord => return BONUS_BOUNDARY,
            _ => {}
        }
    }
    if (prev == Class::Lower && class == Class::Upper)
        || (prev != Class::Number && class == Class::Number)
    {
        return BONUS_CAMEL;
    }
    match class {
        Class::NonWord | Class::Delimiter => BONUS_NONWORD,
        Class::White => BONUS_BOUNDARY_WHITE,
        _ => 0,
    }
}

fn chars_eq(a: char, b: char, case_sensitive: bool) -> bool {
    if a == b {
        return true;
    }
    if case_sensitive {
        return false;
    }
    a.eq_ignore_ascii_case(&b) || a.to_lowercase().next() == b.to_lowercase().next()
}

/// Score `pattern` against `text`. Returns None when the pattern is not a
/// subsequence of the text (under the case rule).
pub fn fuzzy_match(text: &str, pattern: &str, case_sensitive: bool) -> Option<(i32, Vec<usize>)> {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    match_chars(&t, &p, case_sensitive)
}

pub fn match_chars(t: &[char], p: &[char], case_sensitive: bool) -> Option<(i32, Vec<usize>)> {
    let n = t.len();
    let m = p.len();
    if m == 0 {
        return Some((0, Vec::new()));
    }
    if m > n {
        return None;
    }

    // Subsequence prefilter: first occurrence of each pattern char, in order.
    let mut first: Vec<usize> = Vec::with_capacity(m);
    let mut idx = 0usize;
    for &pc in p {
        let mut found = None;
        while idx < n {
            if chars_eq(t[idx], pc, case_sensitive) {
                found = Some(idx);
                break;
            }
            idx += 1;
        }
        let pos = found?;
        first.push(pos);
        idx = pos + 1;
    }
    // Last occurrence of the final pattern char bounds the DP scope.
    let mut last = n - 1;
    while last > 0 && !chars_eq(t[last], p[m - 1], case_sensitive) {
        last -= 1;
    }

    // Huge inputs: score the greedy alignment instead of the full DP (fzf's
    // V1-style fallback). Token picker inputs never hit this.
    if m > 128 || (last - first[0] + 1).saturating_mul(m) > 250_000 {
        let positions = first.clone();
        let score = calculate_score(t, &positions, 0, case_sensitive);
        return Some((score, positions));
    }

    let f0 = first[0];
    let width = last - f0 + 1;
    let bonus: Vec<i32> = t
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            if i == 0 {
                bonus_for(Class::White, class_of(c))
            } else {
                bonus_for(class_of(t[i - 1]), class_of(c))
            }
        })
        .collect();

    let mut h = vec![0i32; m * width];
    let mut c = vec![0i32; m * width];

    // Row 0: pattern[0] with first-char multiplier, gap decay elsewhere.
    let mut in_gap = false;
    let mut prev_h = 0i32;
    for off in 0..width {
        let j = f0 + off;
        if chars_eq(t[j], p[0], case_sensitive) {
            h[off] = SCORE_MATCH + bonus[j] * BONUS_FIRST_MULT;
            c[off] = 1;
            in_gap = false;
        } else {
            let delta = if in_gap {
                SCORE_GAP_EXT
            } else {
                SCORE_GAP_START
            };
            h[off] = (prev_h + delta).max(0);
            c[off] = 0;
            in_gap = true;
        }
        prev_h = h[off];
    }

    // Rows 1..m-1: Smith-Waterman style match/gap transitions.
    for i in 1..m {
        let row = i * width;
        let start = first[i] - f0;
        let mut in_gap = false;
        for off in start..width {
            let j = f0 + off;
            // Left (gap) path; seeded with 0 at the row's first column.
            let left = if off == start { 0 } else { h[row + off - 1] };
            let s2 = left
                + if in_gap {
                    SCORE_GAP_EXT
                } else {
                    SCORE_GAP_START
                };

            let (mut s1, mut consecutive);
            if chars_eq(t[j], p[i], case_sensitive) {
                let diag = h[row - width + off - 1];
                s1 = diag + SCORE_MATCH;
                let mut b = bonus[j];
                consecutive = c[row - width + off - 1] + 1;
                if consecutive > 1 {
                    // fzf: B[col - consecutive + 1]; compute j+1 first so
                    // usize subtraction never underflows.
                    let fb = bonus[j + 1 - consecutive as usize];
                    if b >= BONUS_BOUNDARY && b > fb {
                        consecutive = 1;
                    } else {
                        b = b.max(BONUS_CONSECUTIVE).max(fb);
                    }
                }
                if s1 + b < s2 {
                    s1 += bonus[j];
                    consecutive = 0;
                } else {
                    s1 += b;
                }
            } else {
                s1 = i32::MIN / 2; // sentinel: no match edge
                consecutive = 0;
            }
            c[row + off] = consecutive;
            in_gap = s1 < s2;
            let score = s1.max(s2).max(0);
            h[row + off] = score;
        }
    }

    // Track the max score cell in the last row.
    let last_row = (m - 1) * width;
    let mut max_score = 0i32;
    let mut max_pos = first[m - 1];
    let start_last = first[m - 1] - f0;
    for off in start_last..width {
        let score = h[last_row + off];
        if score > max_score {
            max_score = score;
            max_pos = f0 + off;
        }
    }
    if max_score <= 0 {
        return None;
    }

    // Backtrace (fzf Phase 4) to recover positions.
    let mut positions = Vec::with_capacity(m);
    let mut i = m - 1;
    let mut j = max_pos;
    let mut prefer_match = true;
    loop {
        let j0 = j - f0;
        let s = h[i * width + j0];
        let s1 = if i > 0 && j >= first[i] {
            h[(i - 1) * width + j0 - 1]
        } else {
            0
        };
        let s2 = if j > first[i] {
            h[i * width + j0 - 1]
        } else {
            0
        };
        let row = i;
        if s > s1 && (s > s2 || (s == s2 && prefer_match)) {
            positions.push(j);
            if i == 0 {
                break;
            }
            i -= 1;
        }
        // fzf Phase 4: prefer_match reads the pre-decrement row.
        prefer_match = c[row * width + j0] > 1
            || (row + 1 < m
                && j < last
                && j + 1 >= first[row + 1]
                && c[(row + 1) * width + j0 + 1] > 0);
        if j == 0 {
            break;
        }
        j -= 1;
    }
    positions.reverse();
    Some((max_score, positions))
}

/// Score a known alignment (fzf `calculateScore`): used by the large-input
/// fallback so ranking stays comparable.
fn calculate_score(t: &[char], positions: &[usize], _offset: usize, _case_sensitive: bool) -> i32 {
    let mut score = 0i32;
    let mut in_gap = false;
    let mut consecutive = 0i32;
    let mut first_bonus = 0i32;
    let mut prev_class = Class::White;
    let mut pidx = 0usize;
    for (idx, &ch) in t.iter().enumerate() {
        let class = class_of(ch);
        if pidx < positions.len() && positions[pidx] == idx {
            score += SCORE_MATCH;
            let mut bonus = bonus_for(prev_class, class);
            if consecutive == 0 {
                first_bonus = bonus;
            } else {
                if bonus >= BONUS_BOUNDARY && bonus > first_bonus {
                    first_bonus = bonus;
                }
                bonus = bonus.max(first_bonus).max(BONUS_CONSECUTIVE);
            }
            if pidx == 0 {
                score += bonus * BONUS_FIRST_MULT;
            } else {
                score += bonus;
            }
            in_gap = false;
            consecutive += 1;
            pidx += 1;
        } else {
            if in_gap {
                score += SCORE_GAP_EXT;
            } else {
                score += SCORE_GAP_START;
            }
            in_gap = true;
            consecutive = 0;
            first_bonus = 0;
        }
        prev_class = class;
    }
    score.max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fzf_documented_consecutive_beats_gapped() {
        // fzf README/algo doc: "foobar" vs "foo-bar" on "foob".
        let (a, _) = match_chars(
            &"foobar".chars().collect::<Vec<_>>(),
            &"foob".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap();
        let (b, _) = match_chars(
            &"foo-bar".chars().collect::<Vec<_>>(),
            &"foob".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap();
        assert!(a > b, "consecutive run {a} must beat gapped {b}");
    }

    #[test]
    fn gap_penalty_prefers_tighter_matches() {
        let (tight, _) = match_chars(
            &"a_b".chars().collect::<Vec<_>>(),
            &"ab".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap();
        let loose = match_chars(
            &"a_________b".chars().collect::<Vec<_>>(),
            &"ab".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap()
        .0;
        assert!(tight > loose, "tight {tight} vs loose {loose}");
    }

    #[test]
    fn boundary_and_acronym_ranking() {
        // First chars on word boundaries (after '.', '/') beat mid-word hits.
        let (boundary, _) = match_chars(
            &"api.staging".chars().collect::<Vec<_>>(),
            &"as".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap();
        let (mid, _) = match_chars(
            &"please-x".chars().collect::<Vec<_>>(),
            &"as".chars().collect::<Vec<_>>(),
            false,
        )
        .unwrap();
        assert!(boundary > mid, "boundary {boundary} vs mid {mid}");
    }

    #[test]
    fn smart_case_and_subsequence() {
        assert!(match_chars(
            &"Herdr Extractor".chars().collect::<Vec<_>>(),
            &"hex".chars().collect::<Vec<_>>(),
            false,
        )
        .is_some());
        assert!(match_chars(
            &"Herdr Extractor".chars().collect::<Vec<_>>(),
            &"HX".chars().collect::<Vec<_>>(),
            true,
        )
        .is_none());
        assert!(match_chars(
            &"abc".chars().collect::<Vec<_>>(),
            &"acb".chars().collect::<Vec<_>>(),
            false,
        )
        .is_none());
    }

    #[test]
    fn out_of_order_pattern_positions() {
        // Within one token: subsequence across separators, positions in order.
        let (score, pos) = fuzzy_match("https://staging.example/deploy", "stgdep", false).unwrap();
        let mut sorted = pos.clone();
        sorted.sort_unstable();
        assert_eq!(pos, sorted);
        assert!(score > 0);
        // Across tokens (space = AND): each word matches independently anywhere.
        assert!(fuzzy_match("https://staging.example/deploy", "stg", false).is_some());
        assert!(fuzzy_match("https://staging.example/deploy", "dep", false).is_some());
    }
}
