use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::Value;
use crate::error::{Error, Position, Result};
use crate::lexer::{Kind, MAX_NESTING, Part, Template, Token, lex};

mod patterns;
pub(crate) use patterns::{Case, Pattern};

pub(crate) type Block = Vec<Statement>;

#[derive(Debug)]
pub(crate) struct Statement {
    pub at: Position,
    pub kind: StatementKind,
}

#[derive(Debug)]
pub(crate) enum StatementKind {
    Assign(String, Expression, bool),
    Destructure(Pattern, Expression),
    Function(Arc<Function>, bool),
    Struct(Arc<StructDecl>, bool),
    Enum(String, Vec<String>, bool),
    Match(Expression, Vec<Case>),
    Import(Import, String),
    If(Vec<(Expression, Block)>, Block),
    While(Expression, Block),
    For(String, Expression, Block),
    Try(Block, String, Block),
    Return(Option<Expression>),
    Break,
    Continue,
    Pass,
    Command(Template<Expression>),
    Expression(Expression),
}

#[derive(Debug)]
pub(crate) struct Function {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub body: Block,
}

#[derive(Debug)]
pub(crate) struct Parameter {
    pub name: String,
    pub annotation: Option<String>,
    pub default: Option<Expression>,
}

#[derive(Debug)]
pub(crate) struct StructDecl {
    pub name: String,
    pub fields: Vec<Parameter>,
    pub methods: Vec<Arc<Function>>,
}

#[derive(Debug)]
pub(crate) struct Argument {
    pub name: Option<String>,
    pub value: Expression,
}

#[derive(Debug)]
pub(crate) struct Comprehension {
    pub name: String,
    pub items: Expression,
    pub guard: Option<Expression>,
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
    Interpolated(Vec<Part<Expression>>),
    Command(Template<Expression>),
    Lambda(Arc<Function>),
    Name(String),
    List(Vec<Expression>),
    Map(Vec<(String, Expression)>),
    Call(Box<Expression>, Vec<Argument>),
    Pipe(Box<Expression>, Box<Expression>, Vec<Argument>),
    Field(Box<Expression>, String, bool),
    Index(Box<Expression>, Box<Expression>),
    Slice(
        Box<Expression>,
        Option<Box<Expression>>,
        Option<Box<Expression>>,
    ),
    ListComp(Box<Expression>, Box<Comprehension>),
    MapComp(Box<Expression>, Box<Expression>, Box<Comprehension>),
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
    In,
    Contains,
    Coalesce,
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
        if self.depth >= MAX_NESTING {
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
        self.indented()?;
        self.nested(|parser| parser.block(Kind::Dedent))
    }

    fn indented(&mut self) -> Result<()> {
        self.expect(Kind::Colon, "expected ':' before the block")?;
        self.expect(Kind::Newline, "expected a line break after ':'")?;
        self.expect(Kind::Indent, "expected an indented block")
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
            StatementKind::Function(self.function()?, exported)
        } else if self.keyword("struct") {
            StatementKind::Struct(self.struct_decl()?, exported)
        } else if self.keyword("enum") {
            let name = self.name()?;
            self.indented()?;
            let mut members = Vec::new();
            while !self.take(Kind::Dedent) {
                let member = self.name()?;
                if members.contains(&member) {
                    return Err(self.error(format!("duplicate enum member '{member}'")));
                }
                members.push(member);
                self.end_line()?;
            }
            StatementKind::Enum(name, members, exported)
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
        } else if self.destructuring() {
            let pattern = self.pattern()?;
            self.expect(Kind::Assign, "expected '=' after the pattern")?;
            let value = self.expression(0)?;
            self.end_line()?;
            StatementKind::Destructure(pattern, value)
        } else if self.keyword("match") {
            let subject = self.expression(0)?;
            let cases = self.cases()?;
            StatementKind::Match(subject, cases)
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
            match value.kind {
                ExpressionKind::Command(command) => StatementKind::Command(command),
                _ => StatementKind::Expression(value),
            }
        };
        Ok(Statement { at, kind })
    }

    fn parameter(&mut self) -> Result<Parameter> {
        let name = self.name()?;
        let annotation = self.take(Kind::Colon).then(|| self.name()).transpose()?;
        let default = self
            .take(Kind::Assign)
            .then(|| self.expression(0))
            .transpose()?;
        Ok(Parameter {
            name,
            annotation,
            default,
        })
    }

    fn function(&mut self) -> Result<Arc<Function>> {
        let name = self.name()?;
        self.expect(Kind::LeftParen, "expected '(' after the function name")?;
        let parameters = self.sequence(Kind::RightParen, Self::parameter)?;
        if parameters
            .iter()
            .map(|param| &param.name)
            .collect::<HashSet<_>>()
            .len()
            != parameters.len()
        {
            return Err(self.error("function parameter names must be unique"));
        }
        let outer_loops = std::mem::take(&mut self.loops);
        let body = self.body()?;
        self.loops = outer_loops;
        Ok(Arc::new(Function {
            name,
            parameters,
            body,
        }))
    }

    fn struct_decl(&mut self) -> Result<Arc<StructDecl>> {
        let name = self.name()?;
        self.indented()?;
        let mut names = HashSet::new();
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.take(Kind::Dedent) {
            if self.keyword("fn") {
                let method = self.function()?;
                if !method
                    .parameters
                    .first()
                    .is_some_and(|param| param.name == "self" && param.default.is_none())
                {
                    return Err(self.error("a struct method must start with a self parameter"));
                }
                if !names.insert(method.name.clone()) {
                    return Err(self.error(format!("duplicate struct member '{}'", method.name)));
                }
                methods.push(method);
            } else {
                if self.keyword("pass") {
                    self.end_line()?;
                    continue;
                }
                let field = self.parameter()?;
                if !names.insert(field.name.clone()) {
                    return Err(self.error(format!("duplicate struct member '{}'", field.name)));
                }
                self.end_line()?;
                fields.push(field);
            }
        }
        Ok(Arc::new(StructDecl {
            name,
            fields,
            methods,
        }))
    }

    fn argument(&mut self) -> Result<Argument> {
        let name = if matches!(self.current().kind, Kind::Name(_))
            && matches!(
                self.tokens[self.cursor + 1].kind,
                Kind::Colon | Kind::Assign
            ) {
            let name = self.name()?;
            self.advance();
            Some(name)
        } else {
            None
        };
        Ok(Argument {
            name,
            value: self.expression(0)?,
        })
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
                let arguments = self.sequence(Kind::RightParen, Self::argument)?;
                ExpressionKind::Call(Box::new(left), arguments)
            } else if matches!(self.current().kind, Kind::Dot | Kind::SafeDot) {
                let safe = self.advance() == Kind::SafeDot;
                ExpressionKind::Field(Box::new(left), self.name()?, safe)
            } else if self.take(Kind::LeftBracket) {
                let first = if self.current().kind == Kind::Colon {
                    None
                } else {
                    Some(Box::new(self.expression(0)?))
                };
                if self.take(Kind::Colon) {
                    let end = if self.current().kind == Kind::RightBracket {
                        None
                    } else {
                        Some(Box::new(self.expression(0)?))
                    };
                    self.expect(Kind::RightBracket, "expected ']' after the slice")?;
                    ExpressionKind::Slice(Box::new(left), first, end)
                } else {
                    self.expect(Kind::RightBracket, "expected ']' after the index")?;
                    ExpressionKind::Index(Box::new(left), first.unwrap())
                }
            } else if minimum == 0 && self.take(Kind::Pipe) {
                let stage = self.expression(1)?;
                let (callee, arguments) = match stage.kind {
                    ExpressionKind::Call(callee, arguments) => (callee, arguments),
                    _ => (Box::new(stage), Vec::new()),
                };
                ExpressionKind::Pipe(Box::new(left), callee, arguments)
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
                "not" => ExpressionKind::Unary(Unary::Not, Box::new(self.expression(4)?)),
                _ if !reserved(&name) && self.take(Kind::Arrow) => self.lambda(vec![name])?,
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
            Kind::Interpolated(parts) => ExpressionKind::Interpolated(self.interpolated(parts)?),
            Kind::Command(command) => ExpressionKind::Command(
                command
                    .into_iter()
                    .map(|argument| self.interpolated(argument))
                    .collect::<Result<_>>()?,
            ),
            Kind::Minus => {
                if matches!(&self.current().kind, Kind::Number(text) if text == "9223372036854775808")
                {
                    self.advance();
                    ExpressionKind::Literal(Value::Int(i64::MIN))
                } else {
                    ExpressionKind::Unary(Unary::Negate, Box::new(self.expression(8)?))
                }
            }
            Kind::LeftParen => {
                let start = self.cursor;
                if let Ok(parameters) = self.sequence(Kind::RightParen, Self::name)
                    && self.take(Kind::Arrow)
                {
                    return Ok(Expression {
                        at,
                        kind: self.lambda(parameters)?,
                    });
                }
                self.cursor = start;
                let value = self.expression(0)?;
                self.expect(Kind::RightParen, "expected ')' after the expression")?;
                return Ok(value);
            }
            Kind::LeftBracket => self.list()?,
            Kind::LeftBrace => self.map()?,
            _ => return Err(Error::new("SyntaxError", "expected an expression").at(self.path, at)),
        };
        Ok(Expression { at, kind })
    }

    fn lambda(&mut self, parameters: Vec<String>) -> Result<ExpressionKind> {
        if parameters.iter().collect::<HashSet<_>>().len() != parameters.len() {
            return Err(self.error("function parameter names must be unique"));
        }
        let value = self.expression(0)?;
        Ok(ExpressionKind::Lambda(Arc::new(Function {
            name: "<lambda>".into(),
            parameters: parameters
                .into_iter()
                .map(|name| Parameter {
                    name,
                    annotation: None,
                    default: None,
                })
                .collect(),
            body: vec![Statement {
                at: value.at,
                kind: StatementKind::Return(Some(value)),
            }],
        })))
    }

    fn list(&mut self) -> Result<ExpressionKind> {
        if self.take(Kind::RightBracket) {
            return Ok(ExpressionKind::List(Vec::new()));
        }
        let first = self.expression(0)?;
        if self.keyword("for") {
            return Ok(ExpressionKind::ListComp(
                Box::new(first),
                self.comprehension(Kind::RightBracket)?,
            ));
        }
        let mut items = vec![first];
        if self.take(Kind::Comma) {
            items.extend(self.sequence(Kind::RightBracket, |p| p.expression(0))?);
        } else {
            self.expect(Kind::RightBracket, "expected ',' or ']'")?;
        }
        Ok(ExpressionKind::List(items))
    }

    fn map(&mut self) -> Result<ExpressionKind> {
        if self.take(Kind::RightBrace) {
            return Ok(ExpressionKind::Map(Vec::new()));
        }
        let (key, value) = self.map_entry()?;
        if self.keyword("for") {
            return Ok(ExpressionKind::MapComp(
                Box::new(key),
                Box::new(value),
                self.comprehension(Kind::RightBrace)?,
            ));
        }
        let mut entries = vec![self.literal_entry(key, value)?];
        if self.take(Kind::Comma) {
            entries.extend(self.sequence(Kind::RightBrace, |p| {
                let (key, value) = p.map_entry()?;
                p.literal_entry(key, value)
            })?);
        } else {
            self.expect(Kind::RightBrace, "expected ',' or '}'")?;
        }
        Ok(ExpressionKind::Map(entries))
    }

    fn map_entry(&mut self) -> Result<(Expression, Expression)> {
        let key = self.expression(0)?;
        self.expect(Kind::Colon, "expected ':' after the map key")?;
        Ok((key, self.expression(0)?))
    }

    fn literal_entry(&self, key: Expression, value: Expression) -> Result<(String, Expression)> {
        match key.kind {
            ExpressionKind::Name(key) | ExpressionKind::Literal(Value::String(key)) => {
                Ok((key, value))
            }
            _ => Err(
                Error::new("SyntaxError", "map keys must be names or strings")
                    .at(self.path, key.at),
            ),
        }
    }

    fn comprehension(&mut self, closing: Kind) -> Result<Box<Comprehension>> {
        let name = self.name()?;
        if !self.keyword("in") {
            return Err(self.error("expected 'in' after the comprehension variable"));
        }
        let items = self.expression(0)?;
        let guard = self.keyword("if").then(|| self.expression(0)).transpose()?;
        self.expect(closing, "expected the end of the comprehension")?;
        Ok(Box::new(Comprehension { name, items, guard }))
    }

    fn interpolated(&self, parts: Vec<Part<Vec<Token>>>) -> Result<Vec<Part<Expression>>> {
        parts
            .into_iter()
            .map(|part| match part {
                Part::Text(text) => Ok(Part::Text(text)),
                Part::Expression(tokens) => {
                    let mut parser = Parser {
                        tokens,
                        cursor: 0,
                        path: self.path,
                        depth: self.depth,
                        loops: 0,
                    };
                    let expression = parser.expression(0)?;
                    parser.expect(
                        Kind::Eof,
                        "expected the end of the interpolation expression",
                    )?;
                    Ok(Part::Expression(expression))
                }
            })
            .collect()
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
        Kind::Coalesce => (1, Coalesce),
        Kind::Name(name) if name == "or" => (2, Or),
        Kind::Name(name) if name == "and" => (3, And),
        Kind::Equal => (4, Equal),
        Kind::NotEqual => (4, NotEqual),
        Kind::Name(name) if name == "in" => (5, In),
        Kind::Name(name) if name == "contains" => (5, Contains),
        Kind::Less => (5, Less),
        Kind::LessEqual => (5, LessEqual),
        Kind::Greater => (5, Greater),
        Kind::GreaterEqual => (5, GreaterEqual),
        Kind::Plus => (6, Add),
        Kind::Minus => (6, Subtract),
        Kind::Star => (7, Multiply),
        Kind::Slash => (7, Divide),
        Kind::Percent => (7, Remainder),
        _ => return None,
    })
}
