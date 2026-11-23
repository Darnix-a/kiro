use std::path::Path;

use crate::error::{Error, Position, Result};

pub(crate) const MAX_NESTING: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part<T> {
    Text(String),
    Expression(T),
}

pub(crate) type Template<T> = Vec<Vec<Part<T>>>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Name(String),
    Number(String),
    String(String),
    Interpolated(Vec<Part<Vec<Token>>>),
    Command(Template<Vec<Token>>),
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
    SafeDot,
    Coalesce,
    Ellipsis,
    Bar,
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
    Pipe,
    Arrow,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    pub kind: Kind,
    pub at: Position,
}

pub(crate) fn lex(source: &str, path: &Path) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut indents = vec![0];
    let mut delimiters = Vec::new();
    let mut line_count = 0;

    for line in source.lines() {
        line_count += 1;
        let chars: Vec<_> = line.chars().collect();
        let mut lexer = LineLexer {
            chars: &chars,
            cursor: 0,
            line: line_count,
            path,
        };
        while lexer.take(' ') {}
        let indent = lexer.cursor;
        match lexer.peek() {
            Some('\t') => return Err(lexer.error("use spaces for indentation")),
            None | Some('#') => continue,
            _ => {}
        }

        if delimiters.is_empty() {
            if indent > *indents.last().unwrap() {
                indents.push(indent);
                tokens.push(Token {
                    kind: Kind::Indent,
                    at: Position {
                        line: line_count,
                        column: 1,
                    },
                });
            }
            while indent < *indents.last().unwrap() {
                indents.pop();
                tokens.push(Token {
                    kind: Kind::Dedent,
                    at: lexer.position(),
                });
            }
            if indent != *indents.last().unwrap() {
                return Err(lexer.error("indentation does not match an outer block"));
            }
        }
        while let Some(token) = lexer.token(0)? {
            update_delimiters(&token, &mut delimiters, path)?;
            tokens.push(token);
        }
        if delimiters.is_empty() {
            tokens.push(Token {
                kind: Kind::Newline,
                at: Position {
                    line: line_count,
                    column: chars.len() + 1,
                },
            });
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

fn update_delimiters(token: &Token, stack: &mut Vec<(Kind, Position)>, path: &Path) -> Result<()> {
    let closing = match token.kind {
        Kind::LeftParen => Kind::RightParen,
        Kind::LeftBracket => Kind::RightBracket,
        Kind::LeftBrace => Kind::RightBrace,
        Kind::RightParen | Kind::RightBracket | Kind::RightBrace => {
            return match stack.pop() {
                Some((kind, _)) if kind == token.kind => Ok(()),
                _ => {
                    Err(Error::new("SyntaxError", "unmatched closing delimiter").at(path, token.at))
                }
            };
        }
        _ => return Ok(()),
    };
    stack.push((closing, token.at));
    Ok(())
}

struct LineLexer<'a> {
    chars: &'a [char],
    cursor: usize,
    line: usize,
    path: &'a Path,
}

impl LineLexer<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.cursor).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.cursor += 1;
        Some(ch)
    }

    fn take(&mut self, ch: char) -> bool {
        if self.peek() != Some(ch) {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn position(&self) -> Position {
        Position {
            line: self.line,
            column: self.cursor + 1,
        }
    }

    fn error(&self, message: &str) -> Error {
        Error::new("SyntaxError", message).at(self.path, self.position())
    }

    fn token(&mut self, depth: usize) -> Result<Option<Token>> {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.advance();
        }
        let start = self.cursor;
        let at = self.position();
        let Some(ch) = self.advance() else {
            return Ok(None);
        };
        let kind = match ch {
            '#' => return Ok(None),
            '(' => Kind::LeftParen,
            ')' => Kind::RightParen,
            '[' => Kind::LeftBracket,
            ']' => Kind::RightBracket,
            '{' => Kind::LeftBrace,
            '}' => Kind::RightBrace,
            ',' => Kind::Comma,
            ':' => Kind::Colon,
            '.' if self.chars.get(self.cursor..self.cursor + 2) == Some(&['.', '.']) => {
                self.cursor += 2;
                Kind::Ellipsis
            }
            '.' => Kind::Dot,
            '?' if self.take('.') => Kind::SafeDot,
            '?' if self.take('?') => Kind::Coalesce,
            '|' => Kind::Bar,
            '+' => Kind::Plus,
            '-' if self.take('>') => Kind::Pipe,
            '-' => Kind::Minus,
            '*' => Kind::Star,
            '/' => Kind::Slash,
            '%' => Kind::Percent,
            '=' if self.take('>') => Kind::Arrow,
            '=' if self.take('=') => Kind::Equal,
            '=' => Kind::Assign,
            '!' if self.take('=') => Kind::NotEqual,
            '!' => return Err(Error::new("SyntaxError", "expected '!='").at(self.path, at)),
            '<' if self.take('=') => Kind::LessEqual,
            '<' => Kind::Less,
            '>' if self.take('=') => Kind::GreaterEqual,
            '>' => Kind::Greater,
            '$' => {
                while matches!(self.peek(), Some(' ' | '\t')) {
                    self.advance();
                }
                let Some(delimiter @ ('"' | '\u{60}')) = self.advance() else {
                    return Err(self.error("expected a quoted command after '$'"));
                };
                Kind::Command(self.command(delimiter, depth, at)?)
            }
            '\u{60}' => Kind::Command(self.command(ch, depth, at)?),
            '\'' | '"' => self.string(ch, depth, at)?,
            '0'..='9' => Kind::Number(self.number(start)?),
            ch if ch.is_alphabetic() || ch == '_' => {
                while self
                    .peek()
                    .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
                {
                    self.advance();
                }
                Kind::Name(self.chars[start..self.cursor].iter().collect())
            }
            _ => {
                return Err(
                    Error::new("SyntaxError", format!("unexpected character '{ch}'"))
                        .at(self.path, at),
                );
            }
        };
        Ok(Some(Token { kind, at }))
    }

    fn number(&mut self, start: usize) -> Result<String> {
        self.digits();
        if self.peek() == Some('.')
            && self
                .chars
                .get(self.cursor + 1)
                .is_some_and(char::is_ascii_digit)
        {
            self.advance();
            self.digits();
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.advance();
            if matches!(self.peek(), Some('+' | '-')) {
                self.advance();
            }
            let exponent = self.cursor;
            self.digits();
            if self.cursor == exponent {
                return Err(self.error("expected digits in exponent"));
            }
        }
        Ok(self.chars[start..self.cursor].iter().collect())
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.advance();
        }
    }

    fn string(&mut self, quote: char, depth: usize, at: Position) -> Result<Kind> {
        let mut text = String::new();
        let mut parts = Vec::new();
        while let Some(ch) = self.advance() {
            match ch {
                ch if ch == quote => {
                    return if parts.is_empty() {
                        Ok(Kind::String(text))
                    } else {
                        push_text(&mut parts, &mut text);
                        Ok(Kind::Interpolated(parts))
                    };
                }
                '\\' => {
                    let position = self.position();
                    text.push(match self.advance() {
                        Some('n') => '\n',
                        Some('r') => '\r',
                        Some('t') => '\t',
                        Some(ch @ ('\\' | '"' | '\'' | '{' | '}')) => ch,
                        _ => {
                            return Err(Error::new("SyntaxError", "invalid string escape")
                                .at(self.path, position));
                        }
                    });
                }
                '{' if quote == '"' => {
                    push_text(&mut parts, &mut text);
                    parts.push(Part::Expression(self.interpolation(depth)?));
                }
                _ => text.push(ch),
            }
        }
        Err(Error::new("SyntaxError", "unterminated string literal").at(self.path, at))
    }

    fn interpolation(&mut self, depth: usize) -> Result<Vec<Token>> {
        let opening = Position {
            line: self.line,
            column: self.cursor,
        };
        if depth >= MAX_NESTING {
            return Err(self.error("interpolation nesting exceeds 64 levels"));
        }
        let mut tokens = Vec::new();
        let mut delimiters = Vec::new();
        while let Some(token) = self.token(depth + 1)? {
            if token.kind == Kind::RightBrace && delimiters.is_empty() {
                tokens.push(Token {
                    kind: Kind::Eof,
                    at: token.at,
                });
                return Ok(tokens);
            }
            update_delimiters(&token, &mut delimiters, self.path)?;
            tokens.push(token);
        }
        Err(Error::new("SyntaxError", "unclosed interpolation").at(self.path, opening))
    }

    fn command(
        &mut self,
        delimiter: char,
        depth: usize,
        at: Position,
    ) -> Result<Template<Vec<Token>>> {
        let mut args = Vec::new();
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut quote = None;
        let mut started = false;
        let mut escaped = false;

        while let Some(ch) = self.command_char(delimiter, &mut escaped, at)? {
            if ch.is_whitespace() && quote.is_none() {
                if started {
                    push_text(&mut parts, &mut text);
                    args.push(std::mem::take(&mut parts));
                    started = false;
                }
                continue;
            }
            started = true;
            match ch {
                '\\' if quote == Some('\'') && self.peek() != Some('{') => text.push(ch),
                '\\' if quote == Some('"') && !matches!(self.peek(), Some('"' | '\\' | '{')) => {
                    text.push(ch)
                }
                '\\' => {
                    let next = self
                        .command_char(delimiter, &mut escaped, at)?
                        .ok_or_else(|| self.error("unterminated command escape"))?;
                    text.push(next);
                }
                '\'' | '"' if quote.is_none() => quote = Some(ch),
                '\'' | '"' if quote == Some(ch) => quote = None,
                '{' => {
                    push_text(&mut parts, &mut text);
                    parts.push(Part::Expression(self.interpolation(depth)?));
                }
                _ => text.push(ch),
            }
        }

        if quote.is_some() {
            return Err(
                Error::new("SyntaxError", "unclosed quote inside command").at(self.path, at)
            );
        }
        if started {
            push_text(&mut parts, &mut text);
            args.push(parts);
        }
        if args.is_empty() {
            return Err(Error::new("SyntaxError", "empty command literal").at(self.path, at));
        }
        Ok(args)
    }

    // Handle the outer delimiter before applying quoting and escapes to arguments.
    fn command_char(
        &mut self,
        delimiter: char,
        escaped: &mut bool,
        at: Position,
    ) -> Result<Option<char>> {
        let ch = self.advance().ok_or_else(|| {
            Error::new("SyntaxError", "unterminated command literal").at(self.path, at)
        })?;
        if std::mem::take(escaped) {
            return Ok(Some(ch));
        }
        if ch == delimiter {
            return Ok(None);
        }
        if ch == '\\' {
            if self.take(delimiter) {
                return Ok(Some(delimiter));
            }
            *escaped = true;
        }
        Ok(Some(ch))
    }
}

fn push_text<T>(parts: &mut Vec<Part<T>>, text: &mut String) {
    if !text.is_empty() {
        parts.push(Part::Text(std::mem::take(text)));
    }
}
