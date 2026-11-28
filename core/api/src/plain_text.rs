//! Markdown to plain text for short alerts, which are read aloud by Narrator and VoiceOver.
//! Not a renderer: markers go, the words stay.

/// `markdown` as plain text. Headings, quotes and bullets lose their markers (each list item
/// stays on its own line, numbers stay), `**bold**`, `*italic*`, `_italic_`, `~~strike~~`
/// and `` `code` `` lose their marks, links keep their text and lose the URL, fences and
/// rules vanish, and whitespace collapses. A lone `*` (math, a footnote) stays.
pub fn markdown_to_plain(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for raw in markdown.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            push_line(&mut out, collapse(line));
            continue;
        }
        if line.is_empty() {
            push_line(&mut out, String::new());
            continue;
        }
        let Some(text) = block(line) else { continue };
        push_line(&mut out, collapse(&inline(&text)));
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    out.join("\n")
}

/// The first line of `markdown` worth showing alone, as plain text: a heading-only line ("## Answer")
/// is skipped for the first line of text after it, and is used only when nothing else has words.
/// `None` when there are no words at all.
pub fn first_plain_line(markdown: &str) -> Option<String> {
    let mut heading: Option<String> = None;
    let mut in_fence = false;
    for raw in markdown.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        let text = if in_fence {
            collapse(line)
        } else {
            let Some(text) = block(line) else { continue };
            let text = collapse(&inline(&text));
            if is_heading(line) {
                if heading.is_none() && !text.is_empty() {
                    heading = Some(text);
                }
                continue;
            }
            text
        };
        if !text.is_empty() {
            return Some(text);
        }
    }
    heading
}

/// An ATX heading (`#` to `######` and a space), quoted or not.
fn is_heading(line: &str) -> bool {
    let mut rest = line;
    while let Some(r) = rest.strip_prefix('>') {
        rest = r.trim_start();
    }
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ')
}

/// Keeps one blank line at most, and none for an empty line produced by dropped markup.
fn push_line(out: &mut Vec<String>, line: String) {
    if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
        return;
    }
    out.push(line);
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line without its block marker; `None` for a line that is only markup.
fn block(line: &str) -> Option<String> {
    let mut rest = line;
    if is_rule(rest) || is_table_separator(rest) {
        return None;
    }
    while let Some(r) = rest.strip_prefix('>') {
        rest = r.trim_start();
    }
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        return Some(rest[hashes..].trim().trim_end_matches('#').trim_end().to_string());
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(r) = rest.strip_prefix(bullet) {
            let r = r.trim_start();
            let r = ["[ ] ", "[x] ", "[X] "].iter().find_map(|b| r.strip_prefix(b)).unwrap_or(r);
            return Some(r.to_string());
        }
    }
    if rest.starts_with('|') {
        let cells: Vec<&str> = rest.trim_matches('|').split('|').map(str::trim).filter(|c| !c.is_empty()).collect();
        return Some(cells.join(", "));
    }
    Some(rest.to_string())
}

fn is_rule(line: &str) -> bool {
    let marks: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && ['-', '*', '_'].iter().any(|m| marks.iter().all(|c| c == m))
}

fn is_table_separator(line: &str) -> bool {
    line.contains('-') && line.contains('|') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn inline(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if c.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                out.push(c[i + 1]);
                i += 2;
            }
            '`' => {
                let n = run(&c, i);
                match find_run(&c, i + n, '`', n) {
                    Some(j) => {
                        out.extend(&c[i + n..j]);
                        i = j + n;
                    }
                    None => {
                        out.extend(&c[i..i + n]);
                        i += n;
                    }
                }
            }
            '!' | '[' => {
                let start = if c[i] == '!' && c.get(i + 1) == Some(&'[') { i + 1 } else { i };
                match (c[start] == '[').then(|| link(&c, start)).flatten() {
                    Some((label, end)) => {
                        out.push_str(&inline(&label));
                        i = end;
                    }
                    None => {
                        out.push(c[i]);
                        i += 1;
                    }
                }
            }
            '<' => match c[i + 1..].iter().position(|ch| *ch == '>') {
                Some(p) if is_url(&c[i + 1..i + 1 + p]) => {
                    out.extend(&c[i + 1..i + 1 + p]);
                    i += p + 2;
                }
                _ => {
                    out.push('<');
                    i += 1;
                }
            },
            m @ ('*' | '_' | '~') => {
                let n = run(&c, i);
                let wanted = if m == '~' { n == 2 } else { n <= 3 };
                let prev_word = i > 0 && c[i - 1].is_alphanumeric();
                let opens = wanted && c.get(i + n).is_some_and(|x| !x.is_whitespace()) && (m == '~' || !prev_word);
                let close = if opens { find_close(&c, i + n, m, n) } else { None };
                match close {
                    Some(j) => {
                        out.push_str(&inline(&c[i + n..j].iter().collect::<String>()));
                        i = j + n;
                    }
                    None => {
                        out.extend(&c[i..i + n]);
                        i += n;
                    }
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn run(c: &[char], i: usize) -> usize {
    c[i..].iter().take_while(|x| **x == c[i]).count()
}

/// The start of the next run of exactly `n` of `m` at or after `from`.
fn find_run(c: &[char], from: usize, m: char, n: usize) -> Option<usize> {
    let mut j = from;
    while j < c.len() {
        if c[j] == m {
            let r = run(c, j);
            if r == n {
                return Some(j);
            }
            j += r;
        } else {
            j += 1;
        }
    }
    None
}

/// The closing run of `n` of `m`: not after a space, not before a letter or digit.
fn find_close(c: &[char], from: usize, m: char, n: usize) -> Option<usize> {
    let mut j = from;
    while j < c.len() {
        if c[j] == m {
            let r = run(c, j);
            let after = c.get(j + r);
            if r == n && !c[j - 1].is_whitespace() && !after.is_some_and(|a| a.is_alphanumeric()) {
                return Some(j);
            }
            j += r;
        } else {
            j += 1;
        }
    }
    None
}

/// `[label](url)` at `start`: the label and the index after the `)`.
fn link(c: &[char], start: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (k, ch) in c.iter().enumerate().skip(start) {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0;
    for (k, ch) in c.iter().enumerate().skip(close + 1) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((c[start + 1..close].iter().collect(), k + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn is_url(s: &[char]) -> bool {
    let t: String = s.iter().collect();
    !t.contains(char::is_whitespace) && (t.starts_with("http://") || t.starts_with("https://"))
}

#[cfg(test)]
mod tests {
    use super::markdown_to_plain as plain;

    #[test]
    fn bold_italic_strike_and_code_lose_their_marks() {
        assert_eq!(plain("Ship it **Friday**, _maybe_ *Monday*, ~~never~~ at `noon`."), "Ship it Friday, maybe Monday, never at noon.");
        assert_eq!(plain("***Really*** so"), "Really so");
        assert_eq!(plain("a __b__ c"), "a b c");
    }

    #[test]
    fn the_first_line_skips_a_heading_only_line() {
        use super::first_plain_line as first;
        assert_eq!(first("## Answer\n- **Yes**, see [the deck](https://x.io/d)").as_deref(), Some("Yes, see the deck"));
        assert_eq!(first("# Q3\n\n## Numbers\n\nRevenue is up.").as_deref(), Some("Revenue is up."));
        assert_eq!(first("> ### Quoted\n> words").as_deref(), Some("words"));
        assert_eq!(first("**Yes**, by Friday.\nMore").as_deref(), Some("Yes, by Friday."));
        assert_eq!(first("```\ncode here\n```\nDone").as_deref(), Some("code here"));
        assert_eq!(first("#hashtag first").as_deref(), Some("#hashtag first"), "no space: not a heading");
        assert_eq!(first("## Only a heading\n---").as_deref(), Some("Only a heading"), "nothing else: the heading");
        assert_eq!(first("\n\n"), None);
    }

    #[test]
    fn headings_and_quotes_lose_their_markers() {
        assert_eq!(plain("## Answer\nYes."), "Answer\nYes.");
        assert_eq!(plain("# Title ##"), "Title");
        assert_eq!(plain("> quoted *text*"), "quoted text");
    }

    #[test]
    fn list_items_stay_on_their_own_lines_without_bullets() {
        assert_eq!(plain("- one\n- two\n  * three\n+ four"), "one\ntwo\nthree\nfour");
        assert_eq!(plain("1. first\n2) second"), "1. first\n2) second");
        assert_eq!(plain("- [x] done\n- [ ] todo"), "done\ntodo");
    }

    #[test]
    fn links_keep_their_text_and_drop_the_url() {
        assert_eq!(plain("See [the deck](https://example.com/a_b_(1)) now"), "See the deck now");
        assert_eq!(plain("![chart](https://x.io/c.png) here"), "chart here");
        assert_eq!(plain("Go <https://example.com>"), "Go https://example.com");
        assert_eq!(plain("[**bold** link](http://x)"), "bold link");
        assert_eq!(plain("[not a link] (x) and [broken](oops"), "[not a link] (x) and [broken](oops");
    }

    #[test]
    fn literal_asterisks_and_underscores_stay_unless_they_pair_as_emphasis() {
        assert_eq!(plain("2 * 3 * 4 = 24"), "2 * 3 * 4 = 24");
        assert_eq!(plain("a*b*c and snake_case_name"), "a*b*c and snake_case_name");
        assert_eq!(plain("5 ** 2 and 3 ** 4"), "5 ** 2 and 3 ** 4");
        assert_eq!(plain("a lone * star and a trailing *"), "a lone * star and a trailing *");
        assert_eq!(plain("Cost: 3*"), "Cost: 3*");
    }

    #[test]
    fn code_fences_and_rules_vanish_but_code_text_stays() {
        assert_eq!(plain("Run:\n```sh\ncargo  test\n```\n---\nDone"), "Run:\ncargo test\nDone");
        assert_eq!(plain("`a * b`"), "a * b");
    }

    #[test]
    fn whitespace_collapses_and_blank_lines_keep_one() {
        assert_eq!(plain("\n\n  a   b \n\n\n\nc\n\n"), "a b\n\nc");
        assert_eq!(plain(""), "");
        assert_eq!(plain("---"), "");
    }

    #[test]
    fn non_ascii_survives_and_escapes_unescape() {
        assert_eq!(plain("**Café** → naïve 日本語 😀 \\*not bold\\*"), "Café → naïve 日本語 😀 *not bold*");
    }

    #[test]
    fn tables_become_comma_lines() {
        assert_eq!(plain("| a | b |\n|---|---|\n| 1 | 2 |"), "a, b\n1, 2");
    }
}
