use std::path::Path;

use crate::error::{Error, Position, Result};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Name(String),
    Number(String),
    String(String),
    Newline,
    Indent,
    Dedent,
    Eof,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Colon,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Debug)]
pub(crate) struct Token {
    pub kind: Kind,
    pub at: Position,
}

pub(crate) fn lex(source: &str, path: &Path) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut indents = vec![0];
    let mut delimiters = Vec::new();
    let mut line_count = 0;
    for (line_index, line) in source.lines().enumerate() {
        line_count = line_index + 1;
        let chars: Vec<_> = line.chars().collect();
        let mut cursor = 0;
        while chars.get(cursor) == Some(&' ') {
            cursor += 1;
        }
        let error = |message: &str, column| {
            Error::new("SyntaxError", message).at(
                path,
                Position {
                    line: line_count,
                    column,
                },
            )
        };
        if chars.get(cursor) == Some(&'\t') {
            return Err(error("use spaces for indentation", cursor + 1));
        }
        if matches!(chars.get(cursor), None | Some('#')) {
            continue;
        }
        let mut emit = |kind, column| {
            tokens.push(Token {
                kind,
                at: Position {
                    line: line_count,
                    column,
                },
            });
        };
        if delimiters.is_empty() {
            if cursor > *indents.last().unwrap() {
                indents.push(cursor);
                emit(Kind::Indent, 1);
            }
            while cursor < *indents.last().unwrap() {
                indents.pop();
                emit(Kind::Dedent, cursor + 1);
            }
            if cursor != *indents.last().unwrap() {
                return Err(error(
                    "indentation does not match an outer block",
                    cursor + 1,
                ));
            }
        }
        while let Some(&ch) = chars.get(cursor) {
            let start = cursor;
            cursor += 1;
            let kind = match ch {
                ' ' | '\t' => continue,
                '#' => break,
                '(' | '[' | '{' => {
                    delimiters.push((
                        ch,
                        Position {
                            line: line_count,
                            column: start + 1,
                        },
                    ));
                    match ch {
                        '(' => Kind::LeftParen,
                        '[' => Kind::LeftBracket,
                        _ => Kind::LeftBrace,
                    }
                }
                ')' | ']' | '}' => {
                    let (opening, kind) = match ch {
                        ')' => ('(', Kind::RightParen),
                        ']' => ('[', Kind::RightBracket),
                        _ => ('{', Kind::RightBrace),
                    };
                    if delimiters.pop().map(|(delimiter, _)| delimiter) != Some(opening) {
                        return Err(error("unmatched closing delimiter", start + 1));
                    }
                    kind
                }
                ',' => Kind::Comma,
                ':' => Kind::Colon,
                '.' => Kind::Dot,
                '+' => Kind::Plus,
                '-' => Kind::Minus,
                '*' => Kind::Star,
                '/' => Kind::Slash,
                '%' => Kind::Percent,
                '=' | '!' | '<' | '>' => {
                    let paired = chars.get(cursor) == Some(&'=');
                    if paired {
                        cursor += 1;
                    }
                    match (ch, paired) {
                        ('=', true) => Kind::Equal,
                        ('=', false) => Kind::Assign,
                        ('!', true) => Kind::NotEqual,
                        ('<', true) => Kind::LessEqual,
                        ('<', false) => Kind::Less,
                        ('>', true) => Kind::GreaterEqual,
                        ('>', false) => Kind::Greater,
                        _ => return Err(error("expected '!='", start + 1)),
                    }
                }
                '\'' | '"' => {
                    let quote = ch;
                    let mut value = String::new();
                    loop {
                        let ch = *chars
                            .get(cursor)
                            .ok_or_else(|| error("unterminated string literal", start + 1))?;
                        cursor += 1;
                        if ch == quote {
                            break;
                        }
                        if ch == '\\' {
                            let escape = chars.get(cursor).copied();
                            cursor += 1;
                            value.push(match escape {
                                Some('n') => '\n',
                                Some('r') => '\r',
                                Some('t') => '\t',
                                Some('\\') => '\\',
                                Some('"') => '"',
                                Some('\'') => '\'',
                                _ => return Err(error("invalid string escape", cursor)),
                            });
                        } else {
                            value.push(ch);
                        }
                    }
                    Kind::String(value)
                }
                '0'..='9' => {
                    while chars.get(cursor).is_some_and(char::is_ascii_digit) {
                        cursor += 1;
                    }
                    if chars.get(cursor) == Some(&'.')
                        && chars.get(cursor + 1).is_some_and(char::is_ascii_digit)
                    {
                        cursor += 1;
                        while chars.get(cursor).is_some_and(char::is_ascii_digit) {
                            cursor += 1;
                        }
                    }
                    if matches!(chars.get(cursor), Some('e' | 'E')) {
                        cursor += 1;
                        if matches!(chars.get(cursor), Some('+' | '-')) {
                            cursor += 1;
                        }
                        let exponent = cursor;
                        while chars.get(cursor).is_some_and(char::is_ascii_digit) {
                            cursor += 1;
                        }
                        if cursor == exponent {
                            return Err(error("expected digits in exponent", cursor + 1));
                        }
                    }
                    Kind::Number(chars[start..cursor].iter().collect())
                }
                ch if ch.is_alphabetic() || ch == '_' => {
                    while chars
                        .get(cursor)
                        .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
                    {
                        cursor += 1;
                    }
                    Kind::Name(chars[start..cursor].iter().collect())
                }
                _ => return Err(error(&format!("unexpected character '{ch}'"), start + 1)),
            };
            emit(kind, start + 1);
        }
        if delimiters.is_empty() {
            emit(Kind::Newline, chars.len() + 1);
        }
    }
    if let Some((_, at)) = delimiters.pop() {
        return Err(Error::new("SyntaxError", "unclosed delimiter").at(path, at));
    }
    let at = Position {
        line: line_count + 1,
        column: 1,
    };
    tokens.extend((1..indents.len()).map(|_| Token {
        kind: Kind::Dedent,
        at,
    }));
    tokens.push(Token {
        kind: Kind::Eof,
        at,
    });
    Ok(tokens)
}
