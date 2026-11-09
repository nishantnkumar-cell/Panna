//! Comparing the text of two documents, word by word, all on this computer.
//!
//! Words are matched with a patience diff: words found once in each side anchor the match,
//! and the stretches between anchors are matched the same way, or by longest common
//! subsequence once they're small. Stretches too big for that count as changed.

use std::collections::HashMap;
use std::ops::Range;

/// Most cells in a longest-common-subsequence table.
const MAX_TABLE: usize = 2_000_000;
/// Most changes listed.
const MAX_CHANGES: usize = 20_000;
/// How deep stretches between anchors are split.
const MAX_DEPTH: usize = 64;

/// One difference: text in this document that the other doesn't have, text the other has
/// that this one doesn't, or both.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    /// The page of this document it's on.
    pub page: usize,
    /// The characters of that page's text that changed; empty where text was taken out.
    pub range: Range<usize>,
    /// Those characters.
    pub text: String,
    /// The other document's page with what it has instead, if anything.
    pub other_page: Option<usize>,
    pub other_text: String,
}

impl Change {
    pub fn kind(&self) -> ChangeKind {
        match (self.text.is_empty(), self.other_text.is_empty()) {
            (false, true) => ChangeKind::Added,
            (true, false) => ChangeKind::Removed,
            _ => ChangeKind::Replaced,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChangeKind {
    /// Only in this document.
    Added,
    /// Only in the other.
    Removed,
    /// Different in each.
    Replaced,
}

#[derive(Debug, Clone)]
struct Word {
    page: usize,
    range: Range<usize>,
    text: String,
}

fn words(pages: &[&[char]]) -> Vec<Word> {
    let mut out = Vec::new();
    for (page, chars) in pages.iter().enumerate() {
        let mut start = None;
        for (i, c) in chars.iter().chain([&' ']).enumerate() {
            match (c.is_whitespace(), start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    out.push(Word {
                        page,
                        range: s..i,
                        text: chars.get(s..i).unwrap_or_default().iter().collect(),
                    });
                    start = None;
                }
                _ => {}
            }
        }
    }
    out
}

/// The differences between the text of this document's pages (`ours`) and the other's.
pub fn compare(ours: &[&[char]], theirs: &[&[char]]) -> Vec<Change> {
    let a = words(ours);
    let b = words(theirs);
    let a_text: Vec<&str> = a.iter().map(|w| w.text.as_str()).collect();
    let b_text: Vec<&str> = b.iter().map(|w| w.text.as_str()).collect();
    let mut pairs = Vec::new();
    matches(&a_text, &b_text, 0, 0, 0, &mut pairs);
    pairs.sort_unstable();

    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (ni, nj) in pairs.into_iter().chain([(a.len(), b.len())]) {
        if ni > i || nj > j {
            gap(&a, &b, i..ni, j..nj, &mut changes);
            if changes.len() >= MAX_CHANGES {
                changes.truncate(MAX_CHANGES);
                break;
            }
        }
        (i, j) = (ni + 1, nj + 1);
    }
    changes
}

/// The changes for words `ours` of this document standing where words `theirs` of the other
/// do, one per page of this document they're on.
fn gap(a: &[Word], b: &[Word], ours: Range<usize>, theirs: Range<usize>, out: &mut Vec<Change>) {
    let theirs = b.get(theirs).unwrap_or_default();
    let other_page = theirs.first().map(|w| w.page);
    let mut other_text = theirs
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let next = ours.start;
    let ours = a.get(ours).unwrap_or_default();
    if ours.is_empty() {
        // Taken out: shown just before the next word, or after the last one.
        let at = a
            .get(next)
            .map(|w| (w.page, w.range.start))
            .or_else(|| a.last().map(|w| (w.page, w.range.end)))
            .unwrap_or((0, 0));
        out.push(Change {
            page: at.0,
            range: at.1..at.1,
            text: String::new(),
            other_page,
            other_text,
        });
        return;
    }
    for run in ours.chunk_by(|x, y| x.page == y.page) {
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            continue;
        };
        out.push(Change {
            page: first.page,
            range: first.range.start..last.range.end,
            text: run.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
            other_page,
            other_text: std::mem::take(&mut other_text),
        });
    }
}

/// Adds the matched word pairs of `a` and `b` (offset by `ai` and `bi`) to `out`.
fn matches(a: &[&str], b: &[&str], ai: usize, bi: usize, depth: usize, out: &mut Vec<(usize, usize)>) {
    // The same start and end.
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..prefix).map(|k| (ai + k, bi + k)));
    let (a, b) = (
        a.get(prefix..).unwrap_or_default(),
        b.get(prefix..).unwrap_or_default(),
    );
    let (ai, bi) = (ai + prefix, bi + prefix);
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (
        a.get(..a.len() - suffix).unwrap_or_default(),
        b.get(..b.len() - suffix).unwrap_or_default(),
    );
    out.extend((0..suffix).map(|k| (ai + a.len() + k, bi + b.len() + k)));
    if a.is_empty() || b.is_empty() {
        return;
    }

    let anchors = if depth < MAX_DEPTH {
        unique_anchors(a, b)
    } else {
        Vec::new()
    };
    if anchors.is_empty() {
        if a.len().saturating_mul(b.len()) <= MAX_TABLE {
            lcs(a, b, ai, bi, out);
        }
        return;
    }
    let (mut x, mut y) = (0, 0);
    for (p, q) in anchors.into_iter().chain([(a.len(), b.len())]) {
        if let (Some(sa), Some(sb)) = (a.get(x..p), b.get(y..q)) {
            matches(sa, sb, ai + x, bi + y, depth + 1, out);
        }
        if p < a.len() {
            out.push((ai + p, bi + q));
        }
        (x, y) = (p + 1, q + 1);
    }
}

/// Words found once in each of `a` and `b`, as the longest run of them in the same order.
fn unique_anchors(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let mut counts: HashMap<&str, (u32, usize, u32, usize)> = HashMap::new();
    for (i, w) in a.iter().enumerate() {
        let e = counts.entry(w).or_default();
        e.0 += 1;
        e.1 = i;
    }
    for (j, w) in b.iter().enumerate() {
        if let Some(e) = counts.get_mut(w) {
            e.2 += 1;
            e.3 = j;
        }
    }
    let mut unique: Vec<(usize, usize)> = counts
        .values()
        .filter(|(ca, _, cb, _)| *ca == 1 && *cb == 1)
        .map(|(_, i, _, j)| (*i, *j))
        .collect();
    unique.sort_unstable();
    longest_increasing(&unique)
}

/// The longest run of `pairs` (sorted by their first) whose seconds rise too.
fn longest_increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // Patience sorting: the last pair of the best run of each length, and each pair's
    // predecessor.
    let mut tails: Vec<usize> = Vec::new();
    let mut before: Vec<Option<usize>> = vec![None; pairs.len()];
    for (k, &(_, y)) in pairs.iter().enumerate() {
        let at = tails.partition_point(|&t| pairs.get(t).is_some_and(|p| p.1 < y));
        if at > 0
            && let (Some(slot), Some(&prev)) = (before.get_mut(k), tails.get(at - 1))
        {
            *slot = Some(prev);
        }
        if at == tails.len() {
            tails.push(k);
        } else if let Some(t) = tails.get_mut(at) {
            *t = k;
        }
    }
    let mut run = Vec::new();
    let mut k = tails.last().copied();
    while let Some(i) = k {
        run.extend(pairs.get(i));
        k = before.get(i).copied().flatten();
    }
    run.reverse();
    run
}

/// Longest common subsequence by table, for small stretches.
fn lcs(a: &[&str], b: &[&str], ai: usize, bi: usize, out: &mut Vec<(usize, usize)>) {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    // Lengths of the best match of a[i..] and b[j..].
    let mut table = vec![0u32; (n + 1) * width];
    let cell = |t: &[u32], i: usize, j: usize| t.get(i * width + j).copied().unwrap_or(0);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let best = if a.get(i) == b.get(j) {
                cell(&table, i + 1, j + 1) + 1
            } else {
                cell(&table, i + 1, j).max(cell(&table, i, j + 1))
            };
            if let Some(c) = table.get_mut(i * width + j) {
                *c = best;
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a.get(i) == b.get(j) {
            out.push((ai + i, bi + j));
            i += 1;
            j += 1;
        } else if cell(&table, i + 1, j) >= cell(&table, i, j + 1) {
            i += 1;
        } else {
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ours: &[&str], theirs: &[&str]) -> Vec<Change> {
        let o: Vec<Vec<char>> = ours.iter().map(|s| s.chars().collect()).collect();
        let t: Vec<Vec<char>> = theirs.iter().map(|s| s.chars().collect()).collect();
        let o: Vec<&[char]> = o.iter().map(Vec::as_slice).collect();
        let t: Vec<&[char]> = t.iter().map(Vec::as_slice).collect();
        compare(&o, &t)
    }

    #[test]
    fn the_same_text_has_no_changes() {
        assert!(run(&["a b c", "d e"], &["a b", "c d e"]).is_empty());
    }

    #[test]
    fn added_removed_and_replaced_words_are_found() {
        let changes = run(
            &["The quick red fox jumps over the dog today"],
            &["The quick brown fox jumps over the lazy dog"],
        );
        let summary: Vec<(ChangeKind, &str, &str)> = changes
            .iter()
            .map(|c| (c.kind(), c.text.as_str(), c.other_text.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (ChangeKind::Replaced, "red", "brown"),
                (ChangeKind::Removed, "", "lazy"),
                (ChangeKind::Added, "today", ""),
            ]
        );
        // Where each one is on the page.
        assert_eq!(changes[0].range, 10..13);
        assert_eq!(changes[1].range, 33..33);
        assert_eq!(changes[2].range, 37..42);
    }

    #[test]
    fn changes_say_which_pages_they_are_on() {
        let changes = run(
            &["one two", "three four five"],
            &["one two three", "four six five"],
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].page, 1);
        assert_eq!(changes[0].other_page, Some(1));
        assert_eq!(changes[0].other_text, "six");
        assert_eq!(changes[0].kind(), ChangeKind::Removed);
        // Text taken out at the very end goes after the last word.
        let changes = run(&["a b"], &["a b c"]);
        assert_eq!((changes[0].page, changes[0].range.clone()), (0, 3..3));
    }

    #[test]
    fn moved_paragraphs_and_repeated_words_still_match() {
        let ours = "x y z the the the alpha beta gamma the the";
        let theirs = "alpha beta gamma the the x y z the the the";
        let changes = run(&[ours], &[theirs]);
        let changed: usize = changes
            .iter()
            .map(|c| c.text.split(' ').filter(|w| !w.is_empty()).count())
            .sum();
        assert!(changed <= 6, "{changes:?}");
    }

    #[test]
    fn long_documents_are_compared_quickly() {
        let words: Vec<String> = (0..60_000).map(|i| format!("w{}", i % 5_000)).collect();
        let ours = words.join(" ");
        let mut other = words.clone();
        other[30_000] = "changed".into();
        let theirs = other.join(" ");
        let changes = run(&[&ours], &[&theirs]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].other_text, "changed");
    }
}
