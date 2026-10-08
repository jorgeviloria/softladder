//! Expression language: tokenizer, Pratt parser and evaluator.
//!
//! SoftLadder expressions appear in `Operate` and `Compare` blocks and in SFC
//! transition conditions. The grammar is deliberately small:
//!
//! ```text
//! expr    := or
//! or      := xor ( "OR" xor )*
//! xor     := and ( "XOR" and )*
//! and     := cmp ( "AND" cmp )*
//! cmp     := add ( ( "=" | "<>" | "<" | "<=" | ">" | ">=" ) add )?
//! add     := mul ( ( "+" | "-" ) mul )*
//! mul     := unary ( ( "*" | "/" | "%" ) unary )*
//! unary   := ( "-" | "NOT" ) unary | primary
//! primary := literal | "%"-variable | "(" expr ")"
//! ```
//!
//! Keywords are case-insensitive. Both sides of a binary operator are always
//! evaluated — there is no short-circuiting — so that a scan is deterministic
//! and side-effect free. Division and modulo by zero are reported as
//! [`EvalError::DivideByZero`] rather than panicking.
//!
//! The `%` character is overloaded: it introduces a variable reference when a
//! class letter follows it (`%M0`, `%MW5`) and is the modulo operator
//! otherwise (`9 % 4`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::vars::VarRef;

/// Maximum nesting depth accepted by the parser and the evaluator.
///
/// The limit keeps hostile or generated input from overflowing the stack.
pub const MAX_DEPTH: u32 = 64;

/// A value manipulated by the expression engine.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// Single bit.
    Bit(bool),
    /// 32-bit signed word.
    Word(i32),
    /// 64-bit signed double word.
    DWord(i64),
    /// 64-bit floating point value.
    Real(f64),
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnaryOp {
    /// Arithmetic negation `-x`.
    Neg,
    /// Boolean/bitwise complement `NOT x`.
    Not,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "NOT ",
        })
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinaryOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `AND`
    And,
    /// `OR`
    Or,
    /// `XOR`
    Xor,
}

impl BinaryOp {
    /// Parses a comparison operator written as source text.
    pub fn comparison(text: &str) -> Option<Self> {
        match text {
            "=" | "==" => Some(BinaryOp::Eq),
            "<>" | "!=" => Some(BinaryOp::Ne),
            "<" => Some(BinaryOp::Lt),
            "<=" | "=<" => Some(BinaryOp::Le),
            ">" => Some(BinaryOp::Gt),
            ">=" | "=>" => Some(BinaryOp::Ge),
            _ => None,
        }
    }

    /// `true` when the operator yields a [`Value::Bit`].
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        )
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "<>",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "AND",
            BinaryOp::Or => "OR",
            BinaryOp::Xor => "XOR",
        })
    }
}

/// Expression abstract syntax tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// Literal constant.
    Lit(Value),
    /// Variable reference, resolved through a [`VarSource`].
    Var(VarRef),
    /// Unary operation applied to one operand.
    Unary(UnaryOp, Box<Expr>),
    /// Binary operation applied to two operands.
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Parses `input` into an expression tree.
    pub fn parse(input: &str) -> Result<Self, EvalError> {
        parse(input)
    }

    /// Evaluates this expression against `source`.
    pub fn eval(&self, source: &dyn VarSource) -> Result<Value, EvalError> {
        eval(self, source)
    }
}

impl FromStr for Expr {
    type Err = EvalError;

    fn from_str(input: &str) -> Result<Self, EvalError> {
        parse(input)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(Value::Bit(value)) => write!(f, "{}", u8::from(*value)),
            Expr::Lit(Value::Word(value)) => write!(f, "{value}"),
            Expr::Lit(Value::DWord(value)) => write!(f, "{value}"),
            Expr::Lit(Value::Real(value)) => write!(f, "{value}"),
            Expr::Var(var) => write!(f, "{var}"),
            Expr::Unary(op, inner) => write!(f, "({op}{inner})"),
            Expr::Binary(op, lhs, rhs) => write!(f, "({lhs} {op} {rhs})"),
        }
    }
}

/// Read access to variables during evaluation.
pub trait VarSource {
    /// Returns the current value of `var`, or [`None`] when it is unknown.
    fn get(&self, var: &VarRef) -> Option<Value>;
}

/// Errors produced while parsing or evaluating an expression.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EvalError {
    /// Division or modulo by zero.
    #[error("division by zero")]
    DivideByZero,
    /// Arithmetic overflow that cannot be represented.
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
    /// A referenced variable is not known to the variable source.
    #[error("unknown variable `{0}`")]
    UnknownVar(VarRef),
    /// The expression could not be tokenized or parsed.
    #[error("parse error: {0}")]
    Parse(String),
    /// The operands have incompatible types for the requested operation.
    #[error("type mismatch: {0}")]
    TypeMismatch(&'static str),
    /// The expression nests more deeply than [`MAX_DEPTH`].
    #[error("expression is nested too deeply")]
    TooComplex,
}

impl Value {
    /// Integer view of the value; bits become 0 or 1 and reals truncate.
    pub fn as_i64(self) -> i64 {
        match self {
            Value::Bit(value) => i64::from(value),
            Value::Word(value) => i64::from(value),
            Value::DWord(value) => value,
            Value::Real(value) => value as i64,
        }
    }

    /// Floating point view of the value.
    pub fn as_f64(self) -> f64 {
        match self {
            Value::Bit(value) => f64::from(u8::from(value)),
            Value::Word(value) => f64::from(value),
            Value::DWord(value) => value as f64,
            Value::Real(value) => value,
        }
    }

    /// Boolean view of the value; zero is false and anything else is true.
    pub fn as_bool(self) -> bool {
        match self {
            Value::Bit(value) => value,
            Value::Word(value) => value != 0,
            Value::DWord(value) => value != 0,
            Value::Real(value) => value != 0.0,
        }
    }

    /// `true` when the value is a [`Value::Real`].
    pub fn is_real(self) -> bool {
        matches!(self, Value::Real(_))
    }

    /// Name of the value type, for diagnostics.
    pub fn type_name(self) -> &'static str {
        match self {
            Value::Bit(_) => "bit",
            Value::Word(_) => "word",
            Value::DWord(_) => "dword",
            Value::Real(_) => "real",
        }
    }

    /// Narrows an integer to `Word` when it fits, otherwise keeps it as `DWord`.
    fn narrow(value: i64) -> Value {
        if value >= i64::from(i32::MIN) && value <= i64::from(i32::MAX) {
            Value::Word(value as i32)
        } else {
            Value::DWord(value)
        }
    }

    /// Applies an arithmetic operator.
    pub fn arith(self, other: Value, op: BinaryOp) -> Result<Value, EvalError> {
        if self.is_real() || other.is_real() {
            let (lhs, rhs) = (self.as_f64(), other.as_f64());
            let result = match op {
                BinaryOp::Add => lhs + rhs,
                BinaryOp::Sub => lhs - rhs,
                BinaryOp::Mul => lhs * rhs,
                BinaryOp::Div => {
                    if rhs == 0.0 {
                        return Err(EvalError::DivideByZero);
                    }
                    lhs / rhs
                }
                BinaryOp::Rem => {
                    if rhs == 0.0 {
                        return Err(EvalError::DivideByZero);
                    }
                    lhs % rhs
                }
                _ => return Err(EvalError::TypeMismatch("arithmetic operator")),
            };
            return Ok(Value::Real(result));
        }

        let (lhs, rhs) = (self.as_i64(), other.as_i64());
        let result = match op {
            BinaryOp::Add => lhs.checked_add(rhs),
            BinaryOp::Sub => lhs.checked_sub(rhs),
            BinaryOp::Mul => lhs.checked_mul(rhs),
            BinaryOp::Div => {
                if rhs == 0 {
                    return Err(EvalError::DivideByZero);
                }
                lhs.checked_div(rhs)
            }
            BinaryOp::Rem => {
                if rhs == 0 {
                    return Err(EvalError::DivideByZero);
                }
                lhs.checked_rem(rhs)
            }
            _ => return Err(EvalError::TypeMismatch("arithmetic operator")),
        };
        result
            .map(Value::narrow)
            .ok_or(EvalError::Overflow("arithmetic"))
    }

    /// Applies a comparison operator, yielding a [`Value::Bit`].
    pub fn compare(self, other: Value, op: BinaryOp) -> Result<Value, EvalError> {
        if !op.is_comparison() {
            return Err(EvalError::TypeMismatch("comparison operator"));
        }
        let ordering = if self.is_real() || other.is_real() {
            self.as_f64().partial_cmp(&other.as_f64())
        } else {
            Some(self.as_i64().cmp(&other.as_i64()))
        };
        // Unordered (NaN) comparisons are false.
        let Some(ordering) = ordering else {
            return Ok(Value::Bit(false));
        };
        let result = match op {
            BinaryOp::Eq => ordering.is_eq(),
            BinaryOp::Ne => ordering.is_ne(),
            BinaryOp::Lt => ordering.is_lt(),
            BinaryOp::Le => ordering.is_le(),
            BinaryOp::Gt => ordering.is_gt(),
            BinaryOp::Ge => ordering.is_ge(),
            _ => return Err(EvalError::TypeMismatch("comparison operator")),
        };
        Ok(Value::Bit(result))
    }

    /// Applies `AND`, `OR` or `XOR`, bitwise on integers and logically on bits.
    pub fn logic(self, other: Value, op: BinaryOp) -> Result<Value, EvalError> {
        if !matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
            return Err(EvalError::TypeMismatch("logic operator"));
        }
        if self.is_real() || other.is_real() {
            return Err(EvalError::TypeMismatch("bitwise operator on a real value"));
        }
        if matches!(self, Value::Bit(_)) && matches!(other, Value::Bit(_)) {
            let (lhs, rhs) = (self.as_bool(), other.as_bool());
            return Ok(Value::Bit(match op {
                BinaryOp::And => lhs && rhs,
                BinaryOp::Or => lhs || rhs,
                BinaryOp::Xor => lhs ^ rhs,
                _ => return Err(EvalError::TypeMismatch("logic operator")),
            }));
        }
        let (lhs, rhs) = (self.as_i64(), other.as_i64());
        Ok(Value::narrow(match op {
            BinaryOp::And => lhs & rhs,
            BinaryOp::Or => lhs | rhs,
            BinaryOp::Xor => lhs ^ rhs,
            _ => return Err(EvalError::TypeMismatch("logic operator")),
        }))
    }

    /// Arithmetic negation (`-x`), which is fallible for [`Value::DWord`].
    pub fn negate(self) -> Result<Value, EvalError> {
        match self {
            Value::Bit(value) => Ok(Value::Word(-i32::from(value))),
            Value::Word(value) => Ok(Value::Word(-value)),
            Value::DWord(value) => value
                .checked_neg()
                .map(Value::DWord)
                .ok_or(EvalError::Overflow("negation")),
            Value::Real(value) => Ok(Value::Real(-value)),
        }
    }

    /// Boolean/bitwise complement (`NOT x`).
    pub fn complement(self) -> Result<Value, EvalError> {
        match self {
            Value::Bit(value) => Ok(Value::Bit(!value)),
            Value::Word(value) => Ok(Value::Word(!value)),
            Value::DWord(value) => Ok(Value::DWord(!value)),
            Value::Real(_) => Err(EvalError::TypeMismatch("NOT applied to a real value")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(Value),
    Var(VarRef),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    Not,
    LParen,
    RParen,
}

impl Token {
    /// Binding powers of an infix token, or `None` when it is not infix.
    fn infix(&self) -> Option<(BinaryOp, u8, u8)> {
        let (op, power) = match self {
            Token::Or => (BinaryOp::Or, 1),
            Token::Xor => (BinaryOp::Xor, 2),
            Token::And => (BinaryOp::And, 3),
            Token::Eq => (BinaryOp::Eq, 4),
            Token::Ne => (BinaryOp::Ne, 4),
            Token::Lt => (BinaryOp::Lt, 4),
            Token::Le => (BinaryOp::Le, 4),
            Token::Gt => (BinaryOp::Gt, 4),
            Token::Ge => (BinaryOp::Ge, 4),
            Token::Plus => (BinaryOp::Add, 5),
            Token::Minus => (BinaryOp::Sub, 5),
            Token::Star => (BinaryOp::Mul, 6),
            Token::Slash => (BinaryOp::Div, 6),
            Token::Percent => (BinaryOp::Rem, 6),
            _ => return None,
        };
        Some((op, power, power + 1))
    }
}

/// Binding power of the unary prefix operators.
const UNARY_POWER: u8 = 7;

fn is_var_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'%' | b'.' | b'_' | b'[' | b']')
}

fn tokenize(input: &str) -> Result<Vec<Token>, EvalError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'0'..=b'9' => {
                let start = index;
                let mut real = false;
                while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.')
                {
                    real |= bytes[index] == b'.';
                    index += 1;
                }
                let text = &input[start..index];
                let value =
                    if real {
                        Value::Real(text.parse::<f64>().map_err(|_| {
                            EvalError::Parse(format!("invalid real literal `{text}`"))
                        })?)
                    } else {
                        let number = text.parse::<i64>().map_err(|_| {
                            EvalError::Parse(format!("invalid integer literal `{text}`"))
                        })?;
                        Value::narrow(number)
                    };
                tokens.push(Token::Number(value));
            }
            b'%' => {
                // `%` starts a variable reference only when a class letter
                // follows it; otherwise it is the modulo operator.
                if !matches!(bytes.get(index + 1), Some(next) if next.is_ascii_alphabetic()) {
                    tokens.push(Token::Percent);
                    index += 1;
                    continue;
                }
                let start = index;
                while index < bytes.len() && is_var_char(bytes[index]) {
                    index += 1;
                }
                let text = &input[start..index];
                let var = text
                    .parse::<VarRef>()
                    .map_err(|error| EvalError::Parse(error.to_string()))?;
                tokens.push(Token::Var(var));
            }
            b'+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            b'-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            b'*' => {
                tokens.push(Token::Star);
                index += 1;
            }
            b'/' => {
                tokens.push(Token::Slash);
                index += 1;
            }
            b'(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            b'=' => {
                index += 1;
                if bytes.get(index) == Some(&b'=') {
                    index += 1;
                }
                tokens.push(Token::Eq);
            }
            b'!' => {
                if bytes.get(index + 1) == Some(&b'=') {
                    index += 2;
                    tokens.push(Token::Ne);
                } else {
                    return Err(EvalError::Parse(format!(
                        "unexpected character `!` at byte {index}"
                    )));
                }
            }
            b'<' => {
                index += 1;
                match bytes.get(index) {
                    Some(b'=') => {
                        index += 1;
                        tokens.push(Token::Le);
                    }
                    Some(b'>') => {
                        index += 1;
                        tokens.push(Token::Ne);
                    }
                    _ => tokens.push(Token::Lt),
                }
            }
            b'>' => {
                index += 1;
                if bytes.get(index) == Some(&b'=') {
                    index += 1;
                    tokens.push(Token::Ge);
                } else {
                    tokens.push(Token::Gt);
                }
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let word = input[start..index].to_ascii_uppercase();
                let token = match word.as_str() {
                    "AND" => Token::And,
                    "OR" => Token::Or,
                    "XOR" => Token::Xor,
                    "NOT" => Token::Not,
                    "MOD" => Token::Percent,
                    _ => {
                        return Err(EvalError::Parse(format!("unknown identifier `{word}`")));
                    }
                };
                tokens.push(token);
            }
            _ => {
                return Err(EvalError::Parse(format!(
                    "unexpected character `{}` at byte {index}",
                    byte as char
                )));
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn parse(&mut self) -> Result<Expr, EvalError> {
        let expr = self.expr(0, 0)?;
        if let Some(token) = self.peek() {
            return Err(EvalError::Parse(format!(
                "unexpected trailing token {token:?}"
            )));
        }
        Ok(expr)
    }

    fn expr(&mut self, min_power: u8, depth: u32) -> Result<Expr, EvalError> {
        if depth > MAX_DEPTH {
            return Err(EvalError::TooComplex);
        }
        let mut lhs = match self.advance() {
            Some(Token::Number(value)) => Expr::Lit(value),
            Some(Token::Var(var)) => Expr::Var(var),
            Some(Token::Minus) => {
                Expr::Unary(UnaryOp::Neg, Box::new(self.expr(UNARY_POWER, depth + 1)?))
            }
            Some(Token::Not) => {
                Expr::Unary(UnaryOp::Not, Box::new(self.expr(UNARY_POWER, depth + 1)?))
            }
            Some(Token::LParen) => {
                let inner = self.expr(0, depth + 1)?;
                match self.advance() {
                    Some(Token::RParen) => inner,
                    _ => return Err(EvalError::Parse("missing closing `)`".to_owned())),
                }
            }
            Some(Token::Plus) => return Err(EvalError::Parse("unexpected `+`".to_owned())),
            Some(token) => {
                return Err(EvalError::Parse(format!("unexpected token {token:?}")));
            }
            None => return Err(EvalError::Parse("unexpected end of expression".to_owned())),
        };

        while let Some((op, left_power, right_power)) = self.peek().and_then(Token::infix) {
            if left_power < min_power {
                break;
            }
            self.advance();
            let rhs = self.expr(right_power, depth + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
}

/// Parses `input` into an [`Expr`] tree.
pub fn parse(input: &str) -> Result<Expr, EvalError> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(EvalError::Parse("empty expression".to_owned()));
    }
    Parser {
        tokens,
        position: 0,
    }
    .parse()
}

/// Evaluates `expr` against `source`.
pub fn eval(expr: &Expr, source: &dyn VarSource) -> Result<Value, EvalError> {
    eval_at(expr, source, 0)
}

fn eval_at(expr: &Expr, source: &dyn VarSource, depth: u32) -> Result<Value, EvalError> {
    if depth > MAX_DEPTH {
        return Err(EvalError::TooComplex);
    }
    match expr {
        Expr::Lit(value) => Ok(*value),
        Expr::Var(var) => source
            .get(var)
            .ok_or_else(|| EvalError::UnknownVar(var.clone())),
        Expr::Unary(op, inner) => {
            let value = eval_at(inner, source, depth + 1)?;
            match op {
                UnaryOp::Neg => value.negate(),
                UnaryOp::Not => value.complement(),
            }
        }
        Expr::Binary(op, lhs, rhs) => {
            let left = eval_at(lhs, source, depth + 1)?;
            let right = eval_at(rhs, source, depth + 1)?;
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                    left.arith(right, *op)
                }
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => left.compare(right, *op),
                BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => left.logic(right, *op),
            }
        }
    }
}

/// Applies a comparison operator written as source text to two values.
pub fn compare_values(op: &str, lhs: &Value, rhs: &Value) -> Result<bool, EvalError> {
    let operator = BinaryOp::comparison(op.trim())
        .ok_or_else(|| EvalError::Parse(format!("unknown comparison operator `{op}`")))?;
    Ok(lhs.compare(*rhs, operator)?.as_bool())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestVars(Vec<(VarRef, Value)>);

    impl TestVars {
        fn new(entries: &[(&str, Value)]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(name, value)| {
                        (name.parse::<VarRef>().expect("test variable name"), *value)
                    })
                    .collect(),
            )
        }
    }

    impl VarSource for TestVars {
        fn get(&self, var: &VarRef) -> Option<Value> {
            self.0
                .iter()
                .find(|(candidate, _)| candidate == var)
                .map(|(_, value)| *value)
        }
    }

    fn evaluate(input: &str, vars: &TestVars) -> Result<Value, EvalError> {
        eval(&parse(input)?, vars)
    }

    #[test]
    fn arithmetic_precedence() {
        let vars = TestVars::new(&[]);
        assert_eq!(evaluate("2 + 3 * 4", &vars), Ok(Value::Word(14)));
        assert_eq!(evaluate("(2 + 3) * 4", &vars), Ok(Value::Word(20)));
        assert_eq!(evaluate("10 - 2 - 3", &vars), Ok(Value::Word(5)));
        assert_eq!(evaluate("9 % 4", &vars), Ok(Value::Word(1)));
        assert_eq!(evaluate("-5 + 2", &vars), Ok(Value::Word(-3)));
        assert_eq!(evaluate("2.5 * 2", &vars), Ok(Value::Real(5.0)));
    }

    #[test]
    fn comparisons_and_logic() {
        let vars = TestVars::new(&[
            ("%M0", Value::Bit(true)),
            ("%M1", Value::Bit(false)),
            ("%MW0", Value::Word(7)),
        ]);
        assert_eq!(evaluate("3 < 5", &vars), Ok(Value::Bit(true)));
        assert_eq!(evaluate("5 <= 5", &vars), Ok(Value::Bit(true)));
        assert_eq!(evaluate("4 <> 4", &vars), Ok(Value::Bit(false)));
        assert_eq!(evaluate("%M0 AND %M1", &vars), Ok(Value::Bit(false)));
        assert_eq!(evaluate("%M0 OR %M1", &vars), Ok(Value::Bit(true)));
        assert_eq!(evaluate("NOT %M1", &vars), Ok(Value::Bit(true)));
        assert_eq!(evaluate("%MW0 > 5 AND %M0", &vars), Ok(Value::Bit(true)));
        assert_eq!(evaluate("%MW0 XOR 3", &vars), Ok(Value::Word(4)));
    }

    #[test]
    fn division_by_zero_is_an_error() {
        let vars = TestVars::new(&[]);
        assert_eq!(evaluate("1 / 0", &vars), Err(EvalError::DivideByZero));
        assert_eq!(evaluate("1 % 0", &vars), Err(EvalError::DivideByZero));
        assert_eq!(evaluate("1.0 / 0.0", &vars), Err(EvalError::DivideByZero));
    }

    #[test]
    fn unknown_variables_are_reported() {
        let vars = TestVars::new(&[]);
        assert!(matches!(
            evaluate("%MW0 + 1", &vars),
            Err(EvalError::UnknownVar(_))
        ));
    }

    #[test]
    fn garbage_input_is_a_parse_error() {
        let vars = TestVars::new(&[]);
        for input in ["2 + * 3", "@@", "", "(1 + 2", "1 2", "2 ** 3", "%Z9", "foo"] {
            assert!(
                matches!(evaluate(input, &vars), Err(EvalError::Parse(_))),
                "`{input}` should be a parse error"
            );
        }
    }

    #[test]
    fn type_mismatch_on_real_bitwise() {
        let vars = TestVars::new(&[]);
        assert_eq!(
            evaluate("1.5 OR 2", &vars),
            Err(EvalError::TypeMismatch("bitwise operator on a real value"))
        );
        assert_eq!(
            evaluate("NOT 1.5", &vars),
            Err(EvalError::TypeMismatch("NOT applied to a real value"))
        );
    }

    #[test]
    fn deep_nesting_is_rejected_without_panicking() {
        let input = format!(
            "{}1{}",
            "(".repeat(MAX_DEPTH as usize + 5),
            ")".repeat(MAX_DEPTH as usize + 5)
        );
        assert_eq!(parse(&input), Err(EvalError::TooComplex));
    }

    #[test]
    fn compare_values_helper() {
        assert_eq!(
            compare_values(">=", &Value::Word(3), &Value::Word(3)),
            Ok(true)
        );
        assert_eq!(
            compare_values("<", &Value::Real(1.5), &Value::Word(2)),
            Ok(true)
        );
        assert!(compare_values("~=", &Value::Word(1), &Value::Word(1)).is_err());
    }

    #[test]
    fn expressions_display_round_trip_through_parse() {
        let expr = parse("1 + 2 * 3").expect("expression parses");
        let text = expr.to_string();
        assert_eq!(parse(&text), Ok(expr));
    }
}
