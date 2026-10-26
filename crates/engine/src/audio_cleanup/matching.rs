//! Bounded, order-sensitive take matching. Similarity is textual, not semantic.
use super::{Group, Result, Take, err};

pub(super) const DEFAULT_SIMILARITY: f64 = 80.;
const MAX_WORDS: usize = 100;
const WORK_BUDGET: usize = 25_000_000;

struct Text {
    words: Vec<String>,
    sorted: Vec<String>,
}
impl Text {
    fn new(text: &str) -> Result<Self> {
        if text.len() > 409_600 {
            return Err(err("Take text is too large"));
        }
        let words: Vec<_> =
            text.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).take(MAX_WORDS + 1).map(|s| s.to_lowercase().replace('ё', "е")).collect();
        if words.len() > MAX_WORDS {
            return Err(err("Take contains too many words; select shorter clips"));
        }
        let mut sorted = words.clone();
        sorted.sort();
        Ok(Self { words, sorted })
    }
}
fn spend(budget: &mut usize, amount: usize) -> Result<()> {
    *budget = budget.checked_sub(amount).ok_or_else(|| err("Too many take comparisons; analyze a smaller selection"))?;
    Ok(())
}
fn similarity(a: &Text, b: &Text, minimum: f64, budget: &mut usize) -> Result<Option<f64>> {
    let n = a.words.len();
    let m = b.words.len();
    if n < 3 || m < 3 {
        return Ok(None);
    }
    let longest = n.max(m);
    spend(budget, longest)?;
    let percent = |distance: usize| 100. * (longest - distance) as f64 / longest as f64;
    if percent(n.abs_diff(m)) < minimum {
        return Ok(None);
    }
    if a.words == b.words {
        return Ok(Some(100.));
    }
    // A multiset overlap is an inexpensive upper bound before ordered edit distance.
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while let (Some(x), Some(y)) = (a.sorted.get(i), b.sorted.get(j)) {
        match x.cmp(y) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    if 100. * shared as f64 / (longest as f64) < minimum {
        return Ok(None);
    }
    spend(budget, n * m)?;
    // Text::new bounds both lengths to 100; all row indices below are 0..=100.
    let mut previous = [0usize; MAX_WORDS + 1];
    let mut current = previous;
    for (i, v) in previous.iter_mut().enumerate().take(m + 1) {
        *v = i;
    }
    for (i, x) in a.words.iter().enumerate() {
        current[0] = i + 1;
        for (j, y) in b.words.iter().enumerate() {
            current[j + 1] = (previous[j] + usize::from(x != y)).min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let score = percent(previous[m]);
    Ok((score >= minimum).then_some(score))
}

pub(super) fn group(takes: &[Take], minimum: f64) -> Result<Vec<Group>> {
    if !minimum.is_finite() || !(50.0..=100.0).contains(&minimum) || takes.len() > 2000 {
        return Err(err("Invalid similarity threshold or too many takes"));
    }
    let texts = takes.iter().map(|t| Text::new(&t.text)).collect::<Result<Vec<_>>>()?;
    let mut groups: Vec<(Vec<usize>, f64)> = Vec::new();
    let mut budget = WORK_BUDGET;
    for (index, take) in takes.iter().enumerate() {
        let text = texts.get(index).ok_or_else(|| err("Missing take text"))?;
        if text.words.len() < 3 {
            continue;
        }
        let mut found = false;
        for (members, score) in &mut groups {
            let mut minimum_score = *score;
            let mut matches = true;
            // Every pair must pass, so lowering the threshold cannot create similarity chains.
            for &member in members.iter() {
                let other = takes.get(member).ok_or_else(|| err("Missing take"))?;
                if take.start < other.end && take.end > other.start {
                    matches = false;
                    break;
                }
                let Some(value) = similarity(text, texts.get(member).ok_or_else(|| err("Missing take text"))?, minimum, &mut budget)? else {
                    matches = false;
                    break;
                };
                minimum_score = minimum_score.min(value);
            }
            if matches {
                members.push(index);
                *score = minimum_score;
                found = true;
                break;
            }
        }
        if !found {
            groups.push((vec![index], 100.));
        }
    }
    groups
        .into_iter()
        .filter(|(members, _)| members.len() > 1)
        .map(|(members, score)| {
            let takes = members.into_iter().map(|i| takes.get(i).cloned().ok_or_else(|| err("Missing take"))).collect::<Result<_>>()?;
            Ok(Group { takes, resolved: false, similarity_percent: score })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn score(a: &str, b: &str, threshold: f64) -> Option<f64> {
        let mut budget = WORK_BUDGET;
        similarity(&Text::new(a).unwrap(), &Text::new(b).unwrap(), threshold, &mut budget).unwrap()
    }
    #[test]
    fn punctuation_case_and_yo_are_not_differences() {
        assert_eq!(score("Ещё: один, ДУБЛЬ!", "еще один дубль", 100.), Some(100.));
        assert_eq!(score("Hello—my friend!", "hello, my friend?", 100.), Some(100.));
        assert_eq!(score("Don't change THIS!", "Don’t change this.", 100.), Some(100.));
    }
    #[test]
    fn insertions_deletions_and_substitutions_obey_percentage() {
        let a = "сегодня мы покажем новый редактор";
        assert_eq!(score(a, "сегодня мы покажем крутой редактор", 80.), Some(80.));
        assert!(score(a, "сегодня мы покажем крутой редактор", 81.).is_none());
        assert!(score(a, "сегодня мы вам покажем новый редактор", 83.).is_some());
        assert!(score(a, "сегодня мы покажем редактор", 80.).is_some());
        assert!(score(a, "другие слова никак не совпадают", 50.).is_none());
        assert!(score("один два три", "три два один", 50.).is_none());
        assert!(score("да да", "да да", 50.).is_none());
    }
    #[test]
    fn all_group_members_match_not_just_the_representative() {
        use filmcraft_time::Tick;
        let takes = ["one two three four five", "new two three four five", "one two three four different"]
            .into_iter()
            .enumerate()
            .map(|(i, text)| Take { text: text.into(), start: Tick(i as i64 * 10), end: Tick(i as i64 * 10 + 5) })
            .collect::<Vec<_>>();
        let groups = group(&takes, 80.).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].takes.len(), 2);
        assert_eq!(groups[0].similarity_percent, 80.);
    }
    #[test]
    fn hostile_input_and_exhausted_budget_return_errors() {
        for n in [f64::NAN, f64::INFINITY, -1., 0., 49., 101.] {
            assert!(group(&[], n).is_err());
        }
        assert!(Text::new(&"word ".repeat(101)).is_err());
        assert!(similarity(&Text::new("one two three").unwrap(), &Text::new("one two four").unwrap(), 50., &mut 0).is_err());
    }
}
