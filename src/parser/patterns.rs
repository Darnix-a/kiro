use super::*;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(crate) struct Case {
    pub pattern: Pattern,
    pub guard: Option<Expression>,
    pub body: Block,
}

#[derive(Debug)]
pub(crate) enum Pattern {
    Wildcard,
    Bind(String),
    Constant(Box<Expression>),
    List(Vec<Pattern>, Option<String>),
    Map(Vec<(String, Pattern)>),
    Or(Vec<Pattern>),
}

impl Pattern {
    pub(crate) fn names(&self, names: &mut BTreeSet<String>) {
        match self {
            Self::Bind(name) => {
                names.insert(name.clone());
            }
            Self::List(items, rest) => {
                for item in items {
                    item.names(names);
                }
                names.extend(rest.iter().filter(|name| name.as_str() != "_").cloned());
            }
            Self::Map(items) => {
                for (_, item) in items {
                    item.names(names);
                }
            }
            Self::Or(alternatives) => alternatives[0].names(names),
            _ => {}
        }
    }
}

impl Parser<'_> {
    pub(super) fn cases(&mut self) -> Result<Vec<Case>> {
        self.indented()?;
        self.nested(|p| {
            let mut cases = Vec::new();
            while !p.take(Kind::Dedent) {
                if !p.keyword("case") {
                    return Err(p.error("expected a case in the match block"));
                }
                let pattern = p.pattern()?;
                let guard = p.keyword("if").then(|| p.expression(0)).transpose()?;
                let body = p.body()?;
                cases.push(Case {
                    pattern,
                    guard,
                    body,
                });
            }
            Ok(cases)
        })
    }

    pub(super) fn destructuring(&self) -> bool {
        if !matches!(self.current().kind, Kind::LeftBracket | Kind::LeftBrace) {
            return false;
        }
        let mut depth = 0;
        for (index, token) in self.tokens.iter().enumerate().skip(self.cursor) {
            match token.kind {
                Kind::LeftBracket | Kind::LeftBrace | Kind::LeftParen => depth += 1,
                Kind::RightBracket | Kind::RightBrace | Kind::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return self.tokens[index + 1].kind == Kind::Assign;
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub(super) fn pattern(&mut self) -> Result<Pattern> {
        self.nested(|p| {
            let first = p.pattern_atom()?;
            if !p.take(Kind::Bar) {
                return Ok(first);
            }
            let mut expected = BTreeSet::new();
            first.names(&mut expected);
            let mut alternatives = vec![first];
            loop {
                let alternative = p.pattern_atom()?;
                let mut names = BTreeSet::new();
                alternative.names(&mut names);
                if names != expected {
                    return Err(p.error("all alternatives must bind the same names"));
                }
                alternatives.push(alternative);
                if !p.take(Kind::Bar) {
                    return Ok(Pattern::Or(alternatives));
                }
            }
        })
    }

    fn pattern_atom(&mut self) -> Result<Pattern> {
        match &self.current().kind {
            Kind::LeftBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.take(Kind::RightBracket) {
                    if self.take(Kind::Ellipsis) {
                        let rest = self.name()?;
                        self.take(Kind::Comma);
                        self.expect(Kind::RightBracket, "list rest pattern must be last")?;
                        return Ok(Pattern::List(items, Some(rest)));
                    }
                    items.push(self.pattern()?);
                    if !self.take(Kind::Comma) {
                        self.expect(Kind::RightBracket, "expected ',' or ']'")?;
                        break;
                    }
                }
                Ok(Pattern::List(items, None))
            }
            Kind::LeftBrace => {
                self.advance();
                let entries = self.sequence(Kind::RightBrace, |p| {
                    let (key, shorthand) = match &p.current().kind {
                        Kind::String(text) => (text.clone(), false),
                        Kind::Name(name) => (name.clone(), !reserved(name)),
                        _ => return Err(p.error("expected a map pattern key")),
                    };
                    p.advance();
                    let value = if p.take(Kind::Colon) {
                        p.pattern()?
                    } else if shorthand {
                        if key == "_" {
                            Pattern::Wildcard
                        } else {
                            Pattern::Bind(key.clone())
                        }
                    } else {
                        return Err(p.error("expected ':' after the pattern key"));
                    };
                    Ok((key, value))
                })?;
                Ok(Pattern::Map(entries))
            }
            Kind::LeftParen => {
                self.advance();
                let pattern = self.pattern()?;
                self.expect(Kind::RightParen, "expected ')' after the pattern")?;
                Ok(pattern)
            }
            Kind::Number(_) | Kind::String(_) => Ok(Pattern::Constant(Box::new(self.prefix()?))),
            Kind::Minus if matches!(self.tokens[self.cursor + 1].kind, Kind::Number(_)) => {
                Ok(Pattern::Constant(Box::new(self.prefix()?)))
            }
            Kind::Name(name) if matches!(name.as_str(), "true" | "false" | "null") => {
                Ok(Pattern::Constant(Box::new(self.prefix()?)))
            }
            Kind::Name(_) => {
                let at = self.current().at;
                let name = self.name()?;
                if self.current().kind == Kind::Dot {
                    let mut expression = Expression {
                        at,
                        kind: ExpressionKind::Name(name),
                    };
                    while self.take(Kind::Dot) {
                        expression = Expression {
                            at,
                            kind: ExpressionKind::Field(Box::new(expression), self.name()?, false),
                        };
                    }
                    Ok(Pattern::Constant(Box::new(expression)))
                } else if name == "_" {
                    Ok(Pattern::Wildcard)
                } else {
                    Ok(Pattern::Bind(name))
                }
            }
            _ => Err(self.error("expected a pattern")),
        }
    }
}
