//! Combining what the two UIA strategies read (M3.1 §6: "no single strategy wins").
//!
//! Terminal exposes its buffer only through TextPattern; VS Code's file is only in the
//! cached walk (Monaco's hidden textarea); Word needs both (0.70 and 0.79 recall alone,
//! 0.97 together). Production reads both and keeps the union, but storing both texts
//! whole would double every capture where one is a subset of the other, so a text that
//! adds almost no words to the other is dropped.

use std::collections::HashSet;

/// Share of a text's words the other text must contain for it to count as covered.
const COVERED: f64 = 0.9;

fn words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

fn covered_by(part: &HashSet<String>, whole: &HashSet<String>) -> bool {
    part.is_empty() || part.iter().filter(|w| whole.contains(*w)).count() as f64 / part.len() as f64 >= COVERED
}

/// TextPattern text (`primary`) ∪ cached-walk text (`secondary`).
pub fn union_text(primary: &str, secondary: &str) -> String {
    if secondary.trim().is_empty() {
        return primary.to_string();
    }
    if primary.trim().is_empty() {
        return secondary.to_string();
    }
    let (a, b) = (words(primary), words(secondary));
    if covered_by(&b, &a) {
        primary.to_string()
    } else if covered_by(&a, &b) {
        secondary.to_string()
    } else {
        format!("{primary}\n{secondary}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sides_fall_back_to_the_other() {
        assert_eq!(union_text("", "walk text"), "walk text");
        assert_eq!(union_text("pattern text", "  "), "pattern text");
    }

    #[test]
    fn a_subset_is_dropped() {
        // VS Code: TextPattern sees a slice of the file the walk returns whole.
        let walk = "fn main() { println!(\"hello world\"); let answer = compute(); }";
        assert_eq!(union_text("println hello world", walk), walk);
        // Terminal-like: the walk only adds words the buffer already has.
        assert_eq!(union_text("cargo build finished release profile", "cargo build"), "cargo build finished release profile");
    }

    #[test]
    fn complementary_texts_are_both_kept() {
        // Word: the ribbon is only in the walk, the body only in the pattern.
        let body = "Quarterly planning review for the Windows client";
        let ribbon = "File Home Insert Layout References Review View";
        assert_eq!(union_text(body, ribbon), format!("{body}\n{ribbon}"));
    }
}
