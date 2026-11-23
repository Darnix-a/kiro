use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use crate::Value;
use crate::error::{Error, Position, Result};
use crate::lexer::{Kind, Token, lex};

pub(crate) type Block = Vec<Statement>;

#[derive(Debug)]
pub(crate) struct Statement {
    pub at: Position,
    pub kind: StatementKind,
}

#[derive(Debug)]
pub(crate) enum StatementKind {
    Assign(String, Expression, bool),
    Function(Rc<Function>, bool),
    Import(Import, String),
    If(Vec<(Expression, Block)>, Block),
    While(Expression, Block),
    For(String, Expression, Block),
    Try(Block, String, Block),
    Return(Option<Expression>),
    Break,
    Continue,
    Pass,
    Expression(Expression),
}

#[derive(Debug)]
pub(crate) struct Function {
    pub name: String,
    pub parameters: Vec<String>,
    pub body: Block,
}

#[derive(Debug)]
pub(crate) enum Import {
    Standard(String),
    File(String),
}

#[derive(Debug)]
pub(crate) struct Expression {
    pub at: Position,
    pub kind: ExpressionKind,
}

#[derive(Debug)]
pub(crate) enum ExpressionKind {
    Literal(Value),
    Name(String),
    List(Vec<Expression>),
    Map(Vec<(String, Expression)>),
    Call(Box<Expression>, Vec<Expression>),
    Field(Box<Expression>, String),
    Index(Box<Expression>, Box<Expression>),
    Unary(Unary, Box<Expression>),
    Binary(Box<Expression>, Binary, Box<Expression>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Unary {
    Negate,
    Not,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

pub(crate) fn parse(source: &str, path: &Path) -> Result<Block> {
    let mut parser = Parser {
        tokens: lex(source, path)?,
        cursor: 0,
        path,
        depth: 0,
        loops: 0,
    };
    parser.block(Kind::Eof)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    cursor: usize,
    path: &'a Path,
    depth: usize,
    loops: usize,
}

impl Parser<'_> {
    fn current(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn advance(&mut self) -> Kind {
        let token = self.current().kind.clone();
        if token != Kind::Eof {
            self.cursor += 1;
        }
        token
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error::new("SyntaxError", message).at(self.path, self.current().at)
    }

    fn take(&mut self, kind: Kind) -> bool {
        if self.current().kind == kind {
            self.advance();
            return true;
        }
        false
    }

    fn expect(&mut self, kind: Kind, message: &str) -> Result<()> {
        if self.take(kind) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn keyword(&mut self, name: &str) -> bool {
        if matches!(&self.current().kind, Kind::Name(value) if value == name) {
            self.advance();
            return true;
        }
        false
    }

    fn name(&mut self) -> Result<String> {
        match &self.current().kind {
            Kind::Name(name) if !reserved(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error("expected a name")),
        }
    }

    fn nested<T>(&mut self, parse: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= 64 {
            return Err(self.error("source nesting exceeds 64 levels"));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn block(&mut self, end: Kind) -> Result<Block> {
        let mut statements = Vec::new();
        while self.current().kind != end {
            statements.push(self.statement()?);
        }
        self.advance();
        Ok(statements)
    }

    fn body(&mut self) -> Result<Block> {
        self.expect(Kind::Colon, "expected ':' before the block")?;
        self.expect(Kind::Newline, "expected a line break after ':'")?;
        self.expect(Kind::Indent, "expected an indented block")?;
        self.nested(|parser| parser.block(Kind::Dedent))
    }

    fn end_line(&mut self) -> Result<()> {
        self.expect(Kind::Newline, "expected the end of the line")
    }

    fn statement(&mut self) -> Result<Statement> {
        let at = self.current().at;
        let exported = self.keyword("export");
        if exported && self.depth != 0 {
            return Err(self.error("export is only allowed at file scope"));
        }

        let kind = if self.keyword("fn") {
            let name = self.name()?;
            self.expect(Kind::LeftParen, "expected '(' after the function name")?;
            let parameters = self.sequence(Kind::RightParen, Self::name)?;
            if parameters.iter().collect::<HashSet<_>>().len() != parameters.len() {
                return Err(self.error("function parameter names must be unique"));
            }
            let outer_loops = std::mem::take(&mut self.loops);
            let body = self.body()?;
            self.loops = outer_loops;
            StatementKind::Function(
                Rc::new(Function {
                    name,
                    parameters,
                    body,
                }),
                exported,
            )
        } else if matches!(self.current().kind, Kind::Name(_))
            && self.tokens[self.cursor + 1].kind == Kind::Assign
        {
            let name = self.name()?;
            self.advance();
            let value = self.expression(0)?;
            self.end_line()?;
            StatementKind::Assign(name, value, exported)
        } else if exported {
            return Err(self.error("expected a function or assignment after 'export'"));
        } else if self.keyword("import") {
            let (source, default_name) = if let Kind::String(path) = &self.current().kind {
                let path = path.clone();
                self.advance();
                (Import::File(path), None)
            } else {
                let name = self.name()?;
                (Import::Standard(name.clone()), Some(name))
            };
            let binding = if self.keyword("as") {
                self.name()?
            } else {
                default_name.ok_or_else(|| self.error("a file import requires 'as' and a name"))?
            };
            self.end_line()?;
            StatementKind::Import(source, binding)
        } else if self.keyword("if") {
            let mut branches = vec![(self.expression(0)?, self.body()?)];
            while self.keyword("elif") {
                branches.push((self.expression(0)?, self.body()?));
            }
            let otherwise = if self.keyword("else") {
                self.body()?
            } else {
                Vec::new()
            };
            StatementKind::If(branches, otherwise)
        } else if self.keyword("while") {
            let condition = self.expression(0)?;
            self.loops += 1;
            let body = self.body()?;
            self.loops -= 1;
            StatementKind::While(condition, body)
        } else if self.keyword("for") {
            let name = self.name()?;
            if !self.keyword("in") {
                return Err(self.error("expected 'in' after the loop variable"));
            }
            let items = self.expression(0)?;
            self.loops += 1;
            let body = self.body()?;
            self.loops -= 1;
            StatementKind::For(name, items, body)
        } else if self.keyword("try") {
            let body = self.body()?;
            if !self.keyword("catch") {
                return Err(self.error("expected 'catch' after the try block"));
            }
            let name = self.name()?;
            StatementKind::Try(body, name, self.body()?)
        } else if self.keyword("return") {
            let value = if self.current().kind == Kind::Newline {
                None
            } else {
                Some(self.expression(0)?)
            };
            self.end_line()?;
            StatementKind::Return(value)
        } else if self.keyword("break") {
            if self.loops == 0 {
                return Err(self.error("'break' must be inside a loop"));
            }
            self.end_line()?;
            StatementKind::Break
        } else if self.keyword("continue") {
            if self.loops == 0 {
                return Err(self.error("'continue' must be inside a loop"));
            }
            self.end_line()?;
            StatementKind::Continue
        } else if self.keyword("pass") {
            self.end_line()?;
            StatementKind::Pass
        } else {
            let value = self.expression(0)?;
            self.end_line()?;
            StatementKind::Expression(value)
        };
        Ok(Statement { at, kind })
    }

    fn sequence<T>(
        &mut self,
        end: Kind,
        mut item: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        while !self.take(end.clone()) {
            items.push(item(self)?);
            if !self.take(Kind::Comma) {
                self.expect(end, "expected ',' or a closing delimiter")?;
                break;
            }
        }
        Ok(items)
    }

    fn expression(&mut self, minimum: u8) -> Result<Expression> {
        self.nested(|parser| parser.expression_inner(minimum))
    }

    fn expression_inner(&mut self, minimum: u8) -> Result<Expression> {
        let mut left = self.prefix()?;
        // Bound chains as well as recursive syntax since both can build deep trees.
        for _ in 0..128 {
            let at = self.current().at;
            let kind = if self.take(Kind::LeftParen) {
                let arguments = self.sequence(Kind::RightParen, |p| p.expression(0))?;
                ExpressionKind::Call(Box::new(left), arguments)
            } else if self.take(Kind::Dot) {
                ExpressionKind::Field(Box::new(left), self.name()?)
            } else if self.take(Kind::LeftBracket) {
                let index = self.expression(0)?;
                self.expect(Kind::RightBracket, "expected ']' after the index")?;
                ExpressionKind::Index(Box::new(left), Box::new(index))
            } else if let Some((precedence, op)) = binary(&self.current().kind)
                && precedence >= minimum
            {
                self.advance();
                let right = self.expression(precedence + 1)?;
                ExpressionKind::Binary(Box::new(left), op, Box::new(right))
            } else {
                return Ok(left);
            };
            left = Expression { at, kind };
        }
        Err(self.error("expression chain is too long"))
    }

    fn prefix(&mut self) -> Result<Expression> {
        let at = self.current().at;
        let kind = match self.advance() {
            Kind::Name(name) => match name.as_str() {
                "true" => ExpressionKind::Literal(Value::Bool(true)),
                "false" => ExpressionKind::Literal(Value::Bool(false)),
                "null" => ExpressionKind::Literal(Value::Null),
                "not" => ExpressionKind::Unary(Unary::Not, Box::new(self.expression(3)?)),
                _ if !reserved(&name) => ExpressionKind::Name(name),
                _ => {
                    return Err(
                        Error::new("SyntaxError", format!("unexpected '{name}'")).at(self.path, at)
                    );
                }
            },
            Kind::Number(text) => {
                let value = if text.contains(['.', 'e', 'E']) {
                    text.parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .map(Value::Float)
                } else {
                    text.parse::<i64>().ok().map(Value::Int)
                };
                ExpressionKind::Literal(value.ok_or_else(|| {
                    Error::new("SyntaxError", "number is out of range").at(self.path, at)
                })?)
            }
            Kind::String(value) => ExpressionKind::Literal(value.into()),
            Kind::Minus => {
                if matches!(&self.current().kind, Kind::Number(text) if text == "9223372036854775808")
                {
                    self.advance();
                    ExpressionKind::Literal(Value::Int(i64::MIN))
                } else {
                    ExpressionKind::Unary(Unary::Negate, Box::new(self.expression(7)?))
                }
            }
            Kind::LeftParen => {
                let value = self.expression(0)?;
                self.expect(Kind::RightParen, "expected ')' after the expression")?;
                return Ok(value);
            }
            Kind::LeftBracket => {
                ExpressionKind::List(self.sequence(Kind::RightBracket, |p| p.expression(0))?)
            }
            Kind::LeftBrace => ExpressionKind::Map(self.sequence(Kind::RightBrace, |p| {
                let key = if let Kind::String(key) = &p.current().kind {
                    let key = key.clone();
                    p.advance();
                    key
                } else {
                    p.name()?
                };
                p.expect(Kind::Colon, "expected ':' after the map key")?;
                Ok((key, p.expression(0)?))
            })?),
            _ => return Err(Error::new("SyntaxError", "expected an expression").at(self.path, at)),
        };
        Ok(Expression { at, kind })
    }
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "fn" | "export"
            | "import"
            | "as"
            | "if"
            | "elif"
            | "else"
            | "while"
            | "for"
            | "in"
            | "try"
            | "catch"
            | "return"
            | "break"
            | "continue"
            | "pass"
            | "true"
            | "false"
            | "null"
            | "and"
            | "or"
            | "not"
    )
}

fn binary(kind: &Kind) -> Option<(u8, Binary)> {
    use Binary::*;
    Some(match kind {
        Kind::Name(name) if name == "or" => (1, Or),
        Kind::Name(name) if name == "and" => (2, And),
        Kind::Equal => (3, Equal),
        Kind::NotEqual => (3, NotEqual),
        Kind::Less => (4, Less),
        Kind::LessEqual => (4, LessEqual),
        Kind::Greater => (4, Greater),
        Kind::GreaterEqual => (4, GreaterEqual),
        Kind::Plus => (5, Add),
        Kind::Minus => (5, Subtract),
        Kind::Star => (6, Multiply),
        Kind::Slash => (6, Divide),
        Kind::Percent => (6, Remainder),
        _ => return None,
    })
}
