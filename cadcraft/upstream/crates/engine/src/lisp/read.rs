//! The reader: AutoLISP source text to values. Lists, dotted pairs, `'x` quoting, strings with
//! backslash escapes, integers (a literal beyond 32 bits reads as a real), reals, symbols
//! (upper-cased), `;` line comments and `;| … |;` block comments.
//!
//! It keeps its own stack instead of recursing, so deeply nested input can't overflow it.

use super::LispError;
use super::value::{MAX_DEPTH, Value};

enum Open {
    List { items: Vec<Value>, tail: Option<Value>, dot: bool },
    Quote,
}

/// Every top-level form in `src`.
pub fn read_all(src: &str) -> Result<Vec<Value>, LispError> {
    let mut r = Reader { chars: src.chars().collect(), pos: 0 };
    let mut out = Vec::new();
    while let Some(v) = r.datum()? {
        out.push(v);
    }
    Ok(out)
}

/// The first form in `src` (`read`), nil when there is none.
pub fn read_one(src: &str) -> Result<Value, LispError> {
    let mut r = Reader { chars: src.chars().collect(), pos: 0 };
    Ok(r.datum()?.unwrap_or(Value::Nil))
}

struct Reader {
    chars: Vec<char>,
    pos: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else if c == ';' {
                if self.chars.get(self.pos + 1) == Some(&'|') {
                    // Block comment up to |;
                    self.pos += 2;
                    while self.pos < self.chars.len() && !(self.chars[self.pos] == '|' && self.chars.get(self.pos + 1) == Some(&';')) {
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.chars.len());
                } else {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                }
            } else {
                break;
            }
        }
    }

    /// The next complete datum; `None` at the end of the input.
    fn datum(&mut self) -> Result<Option<Value>, LispError> {
        let mut stack: Vec<Open> = Vec::new();
        loop {
            self.skip_space();
            let Some(c) = self.peek() else {
                if stack.iter().any(|o| matches!(o, Open::List { .. })) {
                    return Err(LispError::new("malformed list on input"));
                }
                if !stack.is_empty() {
                    return Err(LispError::new("malformed quote on input"));
                }
                return Ok(None);
            };
            let done = match c {
                '(' => {
                    self.pos += 1;
                    if stack.len() >= MAX_DEPTH {
                        return Err(LispError::new("too deeply nested input"));
                    }
                    stack.push(Open::List { items: Vec::new(), tail: None, dot: false });
                    continue;
                }
                ')' => {
                    self.pos += 1;
                    match stack.pop() {
                        Some(Open::List { items, tail, dot }) => {
                            if dot && tail.is_none() {
                                return Err(LispError::new("misplaced dot on input"));
                            }
                            Value::list_with_tail(items, tail.unwrap_or(Value::Nil))
                        }
                        Some(Open::Quote) => return Err(LispError::new("malformed quote on input")),
                        // A stray closing parenthesis at top level is ignored, as AutoCAD does.
                        None => continue,
                    }
                }
                '\'' => {
                    self.pos += 1;
                    if stack.len() >= MAX_DEPTH {
                        return Err(LispError::new("too deeply nested input"));
                    }
                    stack.push(Open::Quote);
                    continue;
                }
                '"' => {
                    self.pos += 1;
                    self.string()?
                }
                _ => {
                    let tok = self.token();
                    if tok == "." && matches!(stack.last(), Some(Open::List { items, dot: false, .. }) if !items.is_empty()) {
                        if let Some(Open::List { dot, .. }) = stack.last_mut() {
                            *dot = true;
                        }
                        continue;
                    }
                    atom(&tok)
                }
            };
            // A finished datum: wrap pending quotes, then hand it to the enclosing list.
            let mut v = done;
            loop {
                match stack.last_mut() {
                    Some(Open::Quote) => {
                        stack.pop();
                        v = Value::list(vec![Value::sym("QUOTE"), v]);
                    }
                    Some(Open::List { items, tail, dot }) => {
                        if *dot {
                            if tail.is_some() {
                                return Err(LispError::new("misplaced dot on input"));
                            }
                            *tail = Some(v);
                        } else {
                            items.push(v);
                        }
                        break;
                    }
                    None => return Ok(Some(v)),
                }
            }
        }
    }

    fn string(&mut self) -> Result<Value, LispError> {
        let mut s = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(LispError::new("malformed string on input")) };
            self.pos += 1;
            match c {
                '"' => return Ok(Value::str(&s)),
                '\\' => {
                    let Some(e) = self.peek() else { return Err(LispError::new("malformed string on input")) };
                    self.pos += 1;
                    match e {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'e' => s.push('\u{1b}'),
                        '0'..='7' => {
                            // \nnn: an octal character code.
                            let mut code = e.to_digit(8).unwrap_or(0);
                            for _ in 0..2 {
                                match self.peek().and_then(|d| d.to_digit(8)) {
                                    Some(d) => {
                                        code = code * 8 + d;
                                        self.pos += 1;
                                    }
                                    None => break,
                                }
                            }
                            s.extend(char::from_u32(code));
                        }
                        other => s.push(other),
                    }
                }
                c => s.push(c),
            }
        }
    }

    fn token(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, '(' | ')' | '\'' | '"' | ';') {
                break;
            }
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }
}

/// A number or a symbol.
fn atom(tok: &str) -> Value {
    if let Ok(i) = tok.parse::<i64>() {
        return match i32::try_from(i) {
            Ok(i) => Value::Int(i),
            Err(_) => Value::Real(i as f64),
        };
    }
    let numeric = tok.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        && tok.chars().any(|c| c.is_ascii_digit())
        && tok.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    if numeric && let Ok(r) = tok.parse::<f64>() {
        return Value::Real(r);
    }
    if tok.eq_ignore_ascii_case("nil") {
        return Value::Nil;
    }
    Value::sym(tok)
}

#[cfg(test)]
mod tests {
    use super::super::value::to_text;
    use super::*;

    fn show(src: &str) -> Vec<String> {
        read_all(src).unwrap().iter().map(|v| to_text(v, true)).collect()
    }

    #[test]
    fn reads_forms_atoms_and_comments() {
        assert_eq!(show("(defun c:hi () (princ \"hi\\n\")) ; c\n;| block\n |; 'x"), ["(DEFUN C:HI nil (PRINC \"hi\\n\"))", "'X"]);
        assert_eq!(show("(8 . \"0\") (1 2 . 3) (a .b)"), ["(8 . \"0\")", "(1 2 . 3)", "(A .B)"]);
        assert_eq!(show("1 -2 3.5 .5 1e3 4294967296 nil T"), ["1", "-2", "3.5", "0.5", "1000.0", "4.29497e+009", "nil", "T"]);
        assert_eq!(show("\"a\\\\b\\\"c\\101\""), ["\"a\\\\b\\\"cA\""]);
    }

    #[test]
    fn malformed_and_hostile_input_is_an_error_not_a_crash() {
        assert!(read_all("(1 2").is_err());
        assert!(read_all("\"open").is_err());
        assert!(read_all("(1 . )").is_err());
        assert!(read_all("'").is_err());
        assert!(read_all(&"(".repeat(100_000)).is_err());
        assert!(read_all(&"'".repeat(100_000)).is_err());
        // A stray ) at top level is skipped.
        assert_eq!(show(") 1"), ["1"]);
    }
}
