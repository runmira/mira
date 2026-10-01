//! Splitting a shell command into the operations a policy judges.
//!
//! `cargo test && rm -rf ~` is two commands, and a policy that looks at it
//! as one string gets both directions wrong: a `Bash(rm:*)` deny never
//! fires (the string starts with `cargo`), and a `Bash(cargo test:*)` allow
//! can't fire either (the tail is compound), so harmless chains always ask.
//! Judging each operation on its own is what makes both rules mean what
//! they say.
//!
//! Splits on top-level `&&`, `||`, `;`, `|`, `&` and newlines — never inside
//! quotes, `$(…)`, backticks or `( … )` subshells. Loops and conditionals
//! (`for …; do …; done`) stay one part: their pieces mean nothing alone.

/// The operations in `cmd`, trimmed, in order. A command with no top-level
/// operator yields itself.
pub fn split_compound(cmd: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0usize; // $( … ) and ( … ) nesting
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    let push = |buf: &mut String, parts: &mut Vec<String>| {
        let t = buf.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
        buf.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if let Some(q) = quote {
            buf.push(c);
            if c == '\\' && q != '\'' {
                if let Some(n) = next {
                    buf.push(n);
                    i += 1;
                }
            } else if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                buf.push(c);
                if let Some(n) = next {
                    buf.push(n);
                    i += 1;
                }
            }
            '"' | '\'' | '`' => {
                quote = Some(c);
                buf.push(c);
            }
            '(' => {
                depth += 1;
                buf.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                buf.push(c);
            }
            _ if depth > 0 => buf.push(c),
            '&' if next == Some('&') => {
                push(&mut buf, &mut parts);
                i += 1;
            }
            '|' if next == Some('|') => {
                push(&mut buf, &mut parts);
                i += 1;
            }
            // `>&2` / `2>&1` are redirections, not background jobs.
            '&' if buf.ends_with('>') || next == Some('>') => buf.push(c),
            ';' | '\n' | '|' | '&' => push(&mut buf, &mut parts),
            _ => buf.push(c),
        }
        i += 1;
    }
    push(&mut buf, &mut parts);
    merge_blocks(parts)
}

fn opener_closer(word: &str) -> Option<&'static str> {
    match word {
        "for" | "while" | "until" | "select" => Some("done"),
        "if" => Some("fi"),
        "case" => Some("esac"),
        _ => None,
    }
}

fn is_closer(word: &str) -> bool {
    matches!(word, "done" | "fi" | "esac")
}

/// Rejoin `for …`, `do …`, `done` into the one loop they are.
fn merge_blocks(parts: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut open: Option<String> = None;
    let mut depth = 0i32;
    for p in parts {
        let first = p.split_whitespace().next().unwrap_or("");
        let last = p.split_whitespace().last().unwrap_or("");
        if let Some(acc) = open.as_mut() {
            acc.push_str("; ");
            acc.push_str(&p);
            if opener_closer(first).is_some() {
                depth += 1;
            }
            if is_closer(first) || is_closer(last) {
                depth -= 1;
            }
            if depth <= 0 {
                out.push(open.take().unwrap());
            }
            continue;
        }
        if opener_closer(first).is_some() && !is_closer(last) {
            open = Some(p);
            depth = 1;
            continue;
        }
        out.push(p);
    }
    if let Some(acc) = open {
        out.push(acc);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_split_into_their_operations() {
        assert_eq!(
            split_compound("cargo test && rm -rf ~; ls | head -2 || true"),
            ["cargo test", "rm -rf ~", "ls", "head -2", "true"]
        );
    }

    #[test]
    fn quotes_substitutions_and_subshells_are_never_split() {
        assert_eq!(
            split_compound(r#"echo "a && b" && x=$(ls; pwd) && (cd x; make)"#),
            [r#"echo "a && b""#, "x=$(ls; pwd)", "(cd x; make)"]
        );
        assert_eq!(split_compound("grep 'a|b' f"), ["grep 'a|b' f"]);
    }

    #[test]
    fn redirections_are_not_background_jobs() {
        assert_eq!(split_compound("make 2>&1 | tail"), ["make 2>&1", "tail"]);
        assert_eq!(split_compound("sleep 5 & echo hi"), ["sleep 5", "echo hi"]);
    }

    #[test]
    fn loops_stay_whole() {
        assert_eq!(
            split_compound("for d in a b; do echo $d; done && git status"),
            ["for d in a b; do echo $d; done", "git status"]
        );
        assert_eq!(split_compound("if [ -f x ]; then cat x; fi; ls").len(), 2);
    }

    #[test]
    fn a_plain_command_is_itself() {
        assert_eq!(split_compound("git log -3"), ["git log -3"]);
        assert!(split_compound("  ").is_empty());
    }
}
