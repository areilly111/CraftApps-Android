//! CAD text values as plain text: control codes (`%%d`), Unicode escapes (`\U+00B0`), caret
//! escapes (`^I`) and, in multiline text, the formatting codes (`\P` paragraph breaks, `{\fArial;…}`
//! font changes, `\S1/2;` stacked fractions…).

/// A single-line text value (TEXT, ATTRIB) as plain text.
pub(crate) fn plain(s: &str) -> String {
    percent_codes(&escapes(s))
}

/// A multiline text value (MTEXT) as plain text with `\n` between paragraphs, and the font family
/// its first font change names.
pub(crate) fn mtext(s: &str) -> (String, Option<String>) {
    let s = escapes(s);
    let mut out = String::with_capacity(s.len());
    let mut family = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => {}
            '\\' => match chars.next() {
                Some('P' | 'X' | 'N') => out.push('\n'),
                Some('~') => out.push('\u{A0}'),
                Some(c @ ('\\' | '{' | '}')) => out.push(c),
                Some('L' | 'l' | 'O' | 'o' | 'K' | 'k') => {}
                Some('S') => {
                    // A stacked fraction `\Snum^den;` (or / or #): written as num/den.
                    let arg = until_semicolon(&mut chars);
                    out.push_str(&arg.replace(['^', '#'], "/"));
                }
                Some(f @ ('f' | 'F')) => {
                    let arg = until_semicolon(&mut chars);
                    if family.is_none() && f == 'f' {
                        family = arg.split('|').next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                    }
                }
                // Height, colour, tracking, oblique, width, alignment, paragraph: settings with
                // an argument.
                Some('H' | 'C' | 'c' | 'T' | 'Q' | 'W' | 'A' | 'p') => {
                    until_semicolon(&mut chars);
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    (percent_codes(&out), family)
}

/// The characters up to the next `;` (dropped).
fn until_semicolon(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut arg = String::new();
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
        arg.push(c);
    }
    arg
}

/// `\U+XXXX` Unicode escapes (how files before DXF 2007 hold characters outside their code page)
/// and caret escapes: `^ ` is a caret, `^I` a tab; other control characters are dropped.
fn escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        let after = rest.get(c.len_utf8()..).unwrap_or("");
        if c == '\\'
            && let Some(hex) = after.strip_prefix("U+").or_else(|| after.strip_prefix("u+")).and_then(|h| h.get(..4))
            && let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        {
            out.push(ch);
            rest = after.get(6..).unwrap_or("");
            continue;
        }
        if c == '^'
            && let Some(next) = after.chars().next()
        {
            match next {
                ' ' => out.push('^'),
                'I' => out.push('\t'),
                _ => {}
            }
            rest = after.get(next.len_utf8()..).unwrap_or("");
            continue;
        }
        if !c.is_control() || c == '\t' {
            out.push(c);
        }
        rest = after;
    }
    out
}

/// `%%` control codes: `%%d` degree, `%%p` plus-minus, `%%c` diameter, `%%%` percent, `%%nnn` a
/// character code; the underline, overline and strike-through toggles (`%%u`, `%%o`, `%%k`) are
/// dropped.
fn percent_codes(s: &str) -> String {
    if !s.contains("%%") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("%%") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let code = rest.get(i + 2..).unwrap_or("");
        let mut skip = 1;
        match code.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('d') => out.push('°'),
            Some('p') => out.push('±'),
            Some('c') => out.push('⌀'),
            Some('%') => out.push('%'),
            Some('u' | 'o' | 'k') => {}
            Some(c) if c.is_ascii_digit() => {
                let digits: String = code.chars().take(3).take_while(char::is_ascii_digit).collect();
                skip = digits.len();
                out.extend(digits.parse::<u32>().ok().and_then(char::from_u32).filter(|c| !c.is_control()));
            }
            _ => {
                out.push_str("%%");
                skip = 0;
            }
        }
        rest = code.get(skip..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_codes_and_escapes() {
        assert_eq!(plain("45%%d %%p0.1 %%c20 100%%% %%uunder%%u"), "45° ±0.1 ⌀20 100% under");
        assert_eq!(plain("caf\\U+00E9 ^ x^I^Jy"), "café ^x\ty", "`^ ` is a caret");
        assert_eq!(plain("%%065%%066"), "AB");
        assert_eq!(plain("50%% off"), "50%% off", "an unknown code stays");
        assert_eq!(plain("\\U+"), "\\U+");
    }

    #[test]
    fn mtext_formatting_is_dropped() {
        let (t, family) = mtext("{\\fArial|b1|i0|c0|p34;Title}\\P\\H2.5x;Line \\C1;two\\~\\S1/2;\\Pa\\\\b {\\Lunder}");
        assert_eq!(t, "Title\nLine two\u{A0}1/2\na\\b under");
        assert_eq!(family.as_deref(), Some("Arial"));
        // Carets are written "^ " (a caret before another character is a control character).
        assert_eq!(mtext("x\\S3^ 4;").0, "x3/4");
        assert_eq!(mtext("x\\S3#4;").0, "x3/4");
        assert_eq!(mtext("trailing\\").0, "trailing\\");
    }
}
