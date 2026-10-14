//! A parser for Valve's text KeyValues format (`.vdf`, `.acf`).
//!
//! ```text
//! "AppState"
//! {
//!     "appid"     "220"
//!     "name"      "Half-Life 2"
//! }
//! ```
//!
//! Supports quoted and bare tokens, `\\` `\"` `\n` `\t` escapes, `//` comments and
//! `[$WIN32]`-style conditionals (ignored). Key lookup is case-insensitive, as in Steam.

use std::fmt;

/// Nesting deeper than this is rejected rather than risking the stack.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    /// First value under `key` (case-insensitive). `None` for strings or a missing key.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    /// String value under `key`.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Vdf::as_str)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(entries) => entries,
            Vdf::Str(_) => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Str { text: String, quoted: bool },
    Open,
    Close,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
}

impl Lexer<'_> {
    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            message: message.into(),
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        loop {
            match self.chars.peek().copied() {
                None => return Ok(None),
                Some('\n') => {
                    self.line += 1;
                    self.chars.next();
                }
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.chars.next();
                }
                Some('/') => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'/') {
                        while self.chars.peek().is_some_and(|c| *c != '\n') {
                            self.chars.next();
                        }
                    } else {
                        return Ok(Some(self.bare(String::from("/"))));
                    }
                }
                Some('{') => {
                    self.chars.next();
                    return Ok(Some(Token::Open));
                }
                Some('}') => {
                    self.chars.next();
                    return Ok(Some(Token::Close));
                }
                Some('"') => {
                    self.chars.next();
                    return self.quoted().map(Some);
                }
                Some(_) => return Ok(Some(self.bare(String::new()))),
            }
        }
    }

    fn quoted(&mut self) -> Result<Token, ParseError> {
        let start = self.line;
        let mut text = String::new();
        loop {
            match self.chars.next() {
                None => {
                    return Err(ParseError {
                        line: start,
                        message: "unterminated string".into(),
                    })
                }
                Some('"') => break,
                Some('\\') => match self.chars.next() {
                    Some('\\') => text.push('\\'),
                    Some('"') => text.push('"'),
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some(other) => {
                        text.push('\\');
                        text.push(other);
                    }
                    None => return Err(self.error("unterminated string")),
                },
                Some('\n') => {
                    self.line += 1;
                    text.push('\n');
                }
                Some(c) => text.push(c),
            }
        }
        Ok(Token::Str { text, quoted: true })
    }

    fn bare(&mut self, mut text: String) -> Token {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() || matches!(c, '{' | '}' | '"') {
                break;
            }
            text.push(c);
            self.chars.next();
        }
        Token::Str {
            text,
            quoted: false,
        }
    }
}

/// Parse a whole document. The result is a map of its top-level entries.
pub fn parse(text: &str) -> Result<Vdf, ParseError> {
    let mut lexer = Lexer {
        chars: text.chars().peekable(),
        line: 1,
    };
    let mut pending = None;
    let entries = parse_entries(&mut lexer, &mut pending, 0)?;
    Ok(Vdf::Map(entries))
}

fn is_condition(token: &Token) -> bool {
    matches!(token, Token::Str { text, quoted: false } if text.starts_with('[') && text.ends_with(']'))
}

fn parse_entries(
    lexer: &mut Lexer<'_>,
    pending: &mut Option<Token>,
    depth: usize,
) -> Result<Vec<(String, Vdf)>, ParseError> {
    if depth > MAX_DEPTH {
        return Err(lexer.error("nesting too deep"));
    }
    let mut entries = Vec::new();
    loop {
        let token = match pending.take() {
            Some(t) => Some(t),
            None => lexer.next_token()?,
        };
        let key = match token {
            None if depth == 0 => return Ok(entries),
            None => return Err(lexer.error("unexpected end of file: missing '}'")),
            Some(Token::Close) if depth > 0 => return Ok(entries),
            Some(Token::Close) => return Err(lexer.error("unexpected '}'")),
            Some(Token::Open) => return Err(lexer.error("expected a key, found '{'")),
            Some(Token::Str { text, .. }) => text,
        };
        let value = match lexer.next_token()? {
            Some(Token::Str { text, .. }) => Vdf::Str(text),
            Some(Token::Open) => Vdf::Map(parse_entries(lexer, pending, depth + 1)?),
            Some(Token::Close) => return Err(lexer.error(format!("key {key:?} has no value"))),
            None => return Err(lexer.error(format!("key {key:?} has no value"))),
        };
        entries.push((key, value));

        // An optional `[$PLATFORM]` conditional may follow a value.
        let next = lexer.next_token()?;
        match next {
            Some(ref t) if is_condition(t) => {}
            other => *pending = other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_app_manifest() {
        let text = r#"
"AppState"
{
	"appid"		"220"
	"Universe"		"1"
	"name"		"Half-Life 2"
	"StateFlags"		"4"
	"installdir"		"Half-Life 2"
	"SizeOnDisk"		"6599921433"
	"InstalledDepots"
	{
		"221"
		{
			"manifest"		"123"
		}
	}
}
"#;
        let doc = parse(text).unwrap();
        let app = doc.get("appstate").unwrap();
        assert_eq!(app.get_str("APPID"), Some("220"));
        assert_eq!(app.get_str("name"), Some("Half-Life 2"));
        assert_eq!(
            app.get("InstalledDepots")
                .unwrap()
                .get("221")
                .unwrap()
                .get_str("manifest"),
            Some("123")
        );
        assert_eq!(app.get_str("InstalledDepots"), None);
        assert_eq!(app.get_str("missing"), None);
    }

    #[test]
    fn handles_escapes_comments_and_conditionals() {
        let text = r#"
// a comment
"root"
{
    "path"   "C:\\Program Files (x86)\\Steam"   // trailing comment
    "quote"  "say \"hi\""
    "multi"  "a\tb"
    "unknown" "c:\x"
    bare     token
    "cond"   "yes" [$WIN32]
    "after"  "ok"
}
"#;
        let doc = parse(text).unwrap();
        let root = doc.get("root").unwrap();
        assert_eq!(root.get_str("path"), Some(r"C:\Program Files (x86)\Steam"));
        assert_eq!(root.get_str("quote"), Some(r#"say "hi""#));
        assert_eq!(root.get_str("multi"), Some("a\tb"));
        assert_eq!(root.get_str("unknown"), Some(r"c:\x"));
        assert_eq!(root.get_str("bare"), Some("token"));
        assert_eq!(root.get_str("cond"), Some("yes"));
        assert_eq!(root.get_str("after"), Some("ok"));
    }

    #[test]
    fn keeps_order_and_duplicates_and_returns_the_first() {
        let doc = parse(r#""r" { "k" "1" "k" "2" "other" "3" }"#).unwrap();
        let r = doc.get("r").unwrap();
        assert_eq!(r.entries().len(), 3);
        assert_eq!(r.get_str("k"), Some("1"));
    }

    #[test]
    fn empty_input_and_bom_are_fine() {
        assert_eq!(parse("").unwrap(), Vdf::Map(vec![]));
        assert!(parse("\u{feff}\"a\" \"b\"").unwrap().get("a").is_some());
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        let cases = [
            ("\"a\"\n{\n\"b\" \"c\"\n", "missing '}'", 4),
            ("\"a\" \"unterminated\n\n", "unterminated", 1),
            ("}", "unexpected '}'", 1),
            ("\"a\" { \"b\" }", "has no value", 1),
            ("{ }", "expected a key", 1),
            ("\"a\"", "has no value", 1),
        ];
        for (text, needle, line) in cases {
            let err = parse(text).unwrap_err();
            assert!(err.message.contains(needle), "{text:?}: {err}");
            assert_eq!(err.line, line, "{text:?}: {err}");
        }
    }

    #[test]
    fn rejects_absurd_nesting() {
        let text = "\"a\" {".repeat(100);
        assert!(parse(&text).unwrap_err().message.contains("too deep"));
    }
}
