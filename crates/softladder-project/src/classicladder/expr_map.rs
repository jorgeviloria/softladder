//! Translation between ClassicLadder arithmetic expressions and SoftLadder
//! expression syntax.
//!
//! ClassicLadder writes expressions in a compact, space-free notation that
//! addresses variables as `@<type>/<num>@` and spells its operators `&`, `|`,
//! `:=` and so on; SoftLadder uses the `%`-notation and the words `AND`, `OR`
//! and `=`. The translation is a token rewrite: variables are mapped through
//! the numeric tables of [`super::mapping`], operators are renamed, and the
//! tokens are emitted again with the spacing each side's tokenizer expects
//! (ClassicLadder's evaluator does not skip spaces at all).
//!
//! Anything the reference can express but SoftLadder cannot — a character
//! constant such as `'E'`, a `%SW` system word — is reported through
//! [`Expression::soft`] being `None` and the expression's canonical
//! ClassicLadder text is kept instead.

use softladder_core::expr::Function;
use softladder_core::VarRef;

use super::mapping::{decode_var, encode_var, DecodedVar};

/// Result of translating one ClassicLadder expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expression {
    /// The expression in canonical ClassicLadder spelling, with no spaces.
    pub classic: String,
    /// The expression in SoftLadder spelling, when it has one.
    pub soft: Option<String>,
    /// Why the expression has no SoftLadder spelling.
    pub reason: Option<String>,
    /// Approximations that deserve their own `SL-W030`.
    pub notes: Vec<String>,
}

impl Expression {
    fn failed(classic: String, reason: String) -> Self {
        Self {
            classic,
            soft: None,
            reason: Some(reason),
            notes: Vec::new(),
        }
    }
}

/// Result of translating one ClassicLadder assignment (`ELE_OUTPUT_OPERATE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operate {
    /// The assignment in canonical ClassicLadder spelling, with no spaces.
    pub classic: String,
    /// SoftLadder parameters: `[target, rhs]` when the assignment maps.
    pub params: Option<Vec<String>>,
    /// Why the assignment has no SoftLadder spelling.
    pub reason: Option<String>,
    /// Approximations that deserve their own `SL-W030`.
    pub notes: Vec<String>,
}

/// One lexical token of either notation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// Decimal literal.
    Number(String),
    /// Hexadecimal literal, canonicalised to `$ABC`.
    Hex(String),
    /// ClassicLadder variable, holding the text between the `@` markers.
    ClassicVar(String),
    /// SoftLadder variable, holding the whole `%…` spelling.
    SoftVar(String),
    /// Identifier: a function name or a logical keyword.
    Name(String),
    /// Symbolic operator, canonical for the notation it came from.
    Operator(String),
    /// Character constant, including its quotes.
    Character(String),
}

/// Translates a ClassicLadder expression into SoftLadder syntax.
pub(crate) fn translate_expression(text: &str) -> Expression {
    let Ok(tokens) = tokenize(text) else {
        return Expression::failed(canonical_text(text), "unrecognised characters".to_owned());
    };
    let classic = emit_classic(&tokens);
    match soft_tokens(&tokens) {
        Ok((soft, notes)) => Expression {
            classic,
            soft: Some(soft),
            reason: None,
            notes,
        },
        Err(reason) => Expression {
            classic,
            soft: None,
            reason: Some(reason),
            notes: Vec::new(),
        },
    }
}

/// Translates a ClassicLadder assignment into SoftLadder `[target, rhs]`.
pub(crate) fn translate_operate(text: &str) -> Operate {
    let Ok(tokens) = tokenize(text) else {
        return Operate {
            classic: canonical_text(text),
            params: None,
            reason: Some("unrecognised characters".to_owned()),
            notes: Vec::new(),
        };
    };
    let classic = emit_classic(&tokens);
    let Some(split) = tokens
        .iter()
        .position(|token| matches!(token, Token::Operator(op) if op == ":="))
    else {
        return Operate {
            classic,
            params: None,
            reason: Some("the assignment has no `:=` operator".to_owned()),
            notes: Vec::new(),
        };
    };
    let (target, mut notes) = match &tokens[..split] {
        [Token::ClassicVar(inner)] => match classic_var_to_soft(inner) {
            Ok((var, notes)) => (var, notes),
            Err(reason) => {
                return Operate {
                    classic,
                    params: None,
                    reason: Some(reason),
                    notes: Vec::new(),
                }
            }
        },
        _ => {
            return Operate {
                classic,
                params: None,
                reason: Some("the assignment target is not a single variable".to_owned()),
                notes: Vec::new(),
            }
        }
    };
    match soft_tokens(&tokens[split + 1..]) {
        Ok((rhs, mut rhs_notes)) => {
            notes.append(&mut rhs_notes);
            Operate {
                classic,
                params: Some(vec![target, rhs]),
                reason: None,
                notes,
            }
        }
        Err(reason) => Operate {
            classic,
            params: None,
            reason: Some(reason),
            notes: Vec::new(),
        },
    }
}

/// Renders a SoftLadder expression in ClassicLadder syntax.
///
/// The result always carries canonical ClassicLadder text; `reason` names the
/// part of the expression the reference cannot express, so the exporter can
/// raise `SL-W033`.
pub(crate) fn expression_to_classic(text: &str) -> Expression {
    let Ok(tokens) = tokenize(text) else {
        return Expression {
            classic: text.trim().to_owned(),
            soft: None,
            reason: Some("unrecognised characters".to_owned()),
            notes: Vec::new(),
        };
    };
    let classic = emit_classic(&tokens);
    match classic_tokens(&tokens) {
        Ok((_, notes)) => Expression {
            classic,
            soft: None,
            reason: notes.first().cloned(),
            notes,
        },
        Err(reason) => Expression {
            classic,
            soft: None,
            reason: Some(reason),
            notes: Vec::new(),
        },
    }
}

/// Canonical ClassicLadder spelling of `text` without validating it.
pub(crate) fn canonical_text(text: &str) -> String {
    match tokenize(text) {
        Ok(tokens) => emit_classic(&tokens),
        Err(_) => text.trim().to_owned(),
    }
}

/// Converts tokens to SoftLadder syntax.
fn soft_tokens(tokens: &[Token]) -> Result<(String, Vec<String>), String> {
    let mut output: Vec<(String, bool)> = Vec::new();
    let mut notes = Vec::new();
    for token in tokens {
        match token {
            Token::Number(text) | Token::Hex(text) => output.push((text.clone(), false)),
            Token::ClassicVar(inner) => {
                let (text, mut extra) = classic_var_to_soft(inner)?;
                notes.append(&mut extra);
                output.push((text, false));
            }
            Token::SoftVar(text) => match text.parse::<VarRef>() {
                Ok(var) => output.push((var.to_string(), false)),
                Err(error) => return Err(format!("invalid variable `{text}`: {error}")),
            },
            Token::Name(name) => {
                let upper = name.to_ascii_uppercase();
                match upper.as_str() {
                    "AND" | "OR" | "XOR" | "NOT" => output.push((upper, true)),
                    _ => match Function::from_name(name) {
                        Some(function) => output.push((function.name().to_owned(), true)),
                        None => {
                            return Err(format!(
                                "unknown function or keyword `{name}` has no SoftLadder meaning"
                            ))
                        }
                    },
                }
            }
            Token::Character(text) => {
                return Err(format!(
                    "character constant `{text}` has no SoftLadder equivalent"
                ))
            }
            Token::Operator(op) => {
                let mapped = match op.as_str() {
                    ":=" => "=",
                    "&" => "AND",
                    "|" => "OR",
                    "=>" => ">=",
                    "=<" => "<=",
                    "==" => "=",
                    "!=" => "<>",
                    other => other,
                };
                let is_word = mapped == "AND" || mapped == "OR";
                output.push((mapped.to_owned(), is_word));
            }
        }
    }
    Ok((emit_soft(&output), notes))
}

/// Converts tokens to ClassicLadder syntax.
fn classic_tokens(tokens: &[Token]) -> Result<(String, Vec<String>), String> {
    let mut notes = Vec::new();
    for token in tokens {
        match token {
            Token::SoftVar(text) => {
                let var: VarRef = text
                    .parse()
                    .map_err(|error| format!("invalid variable `{text}`: {error}"))?;
                if let Err(reason) = encode_var(&var) {
                    notes.push(reason);
                }
            }
            Token::Name(name) => {
                let upper = name.to_ascii_uppercase();
                if matches!(upper.as_str(), "AND" | "OR") {
                    continue;
                }
                if upper == "XOR" || upper == "NOT" {
                    notes.push(format!("`{upper}` has no ClassicLadder equivalent"));
                    continue;
                }
                if Function::from_name(name).is_none() {
                    notes.push(format!("`{name}` is not a ClassicLadder function"));
                }
            }
            _ => {}
        }
    }
    Ok((emit_classic(tokens), notes))
}

/// Renders tokens in ClassicLadder syntax (no spaces between tokens).
fn emit_classic(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        let text = match token {
            Token::Number(text) | Token::Hex(text) => text.clone(),
            Token::ClassicVar(inner) => format!("@{inner}@"),
            Token::SoftVar(text) => match text.parse::<VarRef>() {
                Ok(var) => match encode_var(&var) {
                    Ok(encoded) => classic_var_text(&encoded),
                    Err(_) => text.clone(),
                },
                Err(_) => text.clone(),
            },
            Token::Name(name) => {
                let upper = name.to_ascii_uppercase();
                match upper.as_str() {
                    "AND" => "&".to_owned(),
                    "OR" => "|".to_owned(),
                    _ => match Function::from_name(name) {
                        Some(Function::Min) => "MINI".to_owned(),
                        Some(Function::Max) => "MAXI".to_owned(),
                        Some(Function::Avg) => "MOY".to_owned(),
                        Some(function) => function.name().to_owned(),
                        None => name.clone(),
                    },
                }
            }
            Token::Operator(op) => match op.as_str() {
                "==" => "=".to_owned(),
                "!=" => "<>".to_owned(),
                other => other.to_owned(),
            },
            Token::Character(text) => text.clone(),
        };
        // ClassicLadder's evaluator never skips spaces, so tokens are emitted
        // back to back — except where two alphanumeric runs would merge.
        if let (Some(previous), Some(next)) = (out.chars().last(), text.chars().next()) {
            if is_identifier_char(previous) && is_identifier_char(next) {
                out.push(' ');
            }
        }
        out.push_str(&text);
    }
    out
}

/// Renders tokens in SoftLadder syntax, spacing words so its tokenizer can see
/// them as separate tokens.
fn emit_soft(tokens: &[(String, bool)]) -> String {
    let mut out = String::new();
    let mut previous_word = false;
    for (index, (text, is_word)) in tokens.iter().enumerate() {
        let previous_is_open = index > 0 && tokens[index - 1].0 == "(";
        let tight = matches!(text.as_str(), "(" | ")" | ",") || previous_is_open;
        if index > 0 && !tight && (*is_word || previous_word) {
            out.push(' ');
        }
        out.push_str(text);
        previous_word = *is_word;
    }
    out
}

/// `true` for the characters that can appear in a ClassicLadder identifier.
fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// Decodes an `@type/num@` variable into SoftLadder spelling.
fn classic_var_to_soft(inner: &str) -> Result<(String, Vec<String>), String> {
    let Some((var_type, var_num, indexed)) = parse_classic_var(inner) else {
        return Err(format!("`@{inner}@` is not a valid ClassicLadder variable"));
    };
    let mut notes = Vec::new();
    let mut var = match decode_var(var_type, var_num) {
        DecodedVar::Plain(var) => var,
        DecodedVar::Deprecated(var) => {
            notes.push(format!(
                "`@{inner}@` belongs to a deprecated ClassicLadder variable family"
            ));
            var
        }
        DecodedVar::Unsupported => {
            return Err(format!(
                "variable type {var_type} (`@{inner}@`) has no SoftLadder equivalent"
            ));
        }
    };
    if let Some((index_type, index_num)) = indexed {
        if var_num != 0 {
            notes.push(format!(
                "indexed variable `@{inner}@` has base {var_num}, which SoftLadder cannot express"
            ));
        }
        let index = match decode_var(index_type, index_num) {
            DecodedVar::Plain(index) | DecodedVar::Deprecated(index) => index,
            DecodedVar::Unsupported => {
                return Err(format!(
                    "index variable type {index_type} (`@{inner}@`) has no SoftLadder equivalent"
                ));
            }
        };
        if index.index_expr.is_some() {
            return Err(format!("`@{inner}@` nests its index variable too deeply"));
        }
        var = var.with_index_var(index);
    }
    Ok((var.to_string(), notes))
}

/// A `(VarType, VarNum, optional (IndexedVarType, IndexedVarNum))` triple.
type ClassicVar = (i64, i64, Option<(i64, i64)>);

/// Parses the text between the `@` markers of a ClassicLadder variable.
fn parse_classic_var(inner: &str) -> Option<ClassicVar> {
    let (head, index) = match inner.split_once('[') {
        None => (inner, None),
        Some((head, rest)) => {
            let rest = rest.strip_suffix(']')?;
            let (index_type, index_num) = rest.split_once('/')?;
            (
                head,
                Some((parse_number(index_type)?, parse_number(index_num)?)),
            )
        }
    };
    let (var_type, var_num) = head.split_once('/')?;
    Some((parse_number(var_type)?, parse_number(var_num)?, index))
}

/// Renders an encoded variable back into `@type/num@` form.
fn classic_var_text(encoded: &super::mapping::EncodedVar) -> String {
    let mut text = format!("@{}/{}", encoded.var_type, encoded.var_num);
    if let Some((index_type, index_num)) = encoded.indexed {
        text.push_str(&format!("[{index_type}/{index_num}]"));
    }
    text.push('@');
    text
}

/// Parses a decimal number, tolerating surrounding spaces.
fn parse_number(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok()
}

/// Splits expression text into tokens, accepting both notations.
fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    if !text.is_ascii() {
        return Err("expression is not ASCII".to_owned());
    }
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if byte == b'@' {
            let rest = &text[index + 1..];
            let end = rest
                .find('@')
                .ok_or_else(|| "unterminated `@` variable".to_owned())?;
            tokens.push(Token::ClassicVar(rest[..end].to_owned()));
            index += end + 2;
            continue;
        }
        if byte == b'%' {
            if let Some(length) = scan_soft_var(&text[index..]) {
                tokens.push(Token::SoftVar(text[index..index + length].to_owned()));
                index += length;
                continue;
            }
            tokens.push(Token::Operator("%".to_owned()));
            index += 1;
            continue;
        }
        if byte == b'$' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if end == start {
                return Err("`$` is not followed by hexadecimal digits".to_owned());
            }
            tokens.push(Token::Hex(format!(
                "${}",
                text[start..end].to_ascii_uppercase()
            )));
            index = end;
            continue;
        }
        if byte == b'0'
            && matches!(bytes.get(index + 1), Some(b'x') | Some(b'X'))
            && matches!(bytes.get(index + 2), Some(next) if next.is_ascii_hexdigit())
        {
            let start = index + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            tokens.push(Token::Hex(format!(
                "${}",
                text[start..end].to_ascii_uppercase()
            )));
            index = end;
            continue;
        }
        if byte.is_ascii_digit() {
            let mut end = index;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit()
                    || (bytes[end] == b'.'
                        && matches!(bytes.get(end + 1), Some(next) if next.is_ascii_digit())))
            {
                end += 1;
            }
            tokens.push(Token::Number(text[index..end].to_owned()));
            index = end;
            continue;
        }
        if byte == b'\'' {
            let rest = &text[index + 1..];
            let end = rest
                .find('\'')
                .ok_or_else(|| "unterminated character constant".to_owned())?;
            tokens.push(Token::Character(text[index..index + end + 2].to_owned()));
            index += end + 2;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let mut end = index;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            tokens.push(Token::Name(text[index..end].to_owned()));
            index = end;
            continue;
        }
        let two = bytes.get(index + 1).copied();
        let operator = match (byte, two) {
            (b':', Some(b'=')) => Some(":="),
            (b'<', Some(b'=')) => Some("<="),
            (b'>', Some(b'=')) => Some(">="),
            (b'<', Some(b'>')) => Some("<>"),
            (b'=', Some(b'>')) => Some("=>"),
            (b'=', Some(b'<')) => Some("=<"),
            (b'=', Some(b'=')) => Some("=="),
            (b'!', Some(b'=')) => Some("!="),
            (b'+', _) => Some("+"),
            (b'-', _) => Some("-"),
            (b'*', _) => Some("*"),
            (b'/', _) => Some("/"),
            (b'%', _) => Some("%"),
            (b'=', _) => Some("="),
            (b'<', _) => Some("<"),
            (b'>', _) => Some(">"),
            (b'&', _) => Some("&"),
            (b'|', _) => Some("|"),
            (b'(', _) => Some("("),
            (b')', _) => Some(")"),
            (b',', _) => Some(","),
            _ => None,
        };
        let Some(operator) = operator else {
            return Err(format!("unexpected character `{}`", byte as char));
        };
        let length = if operator.len() == 2 { 2 } else { 1 };
        tokens.push(Token::Operator(operator.to_owned()));
        index += length;
    }
    Ok(tokens)
}

/// Length of the longest `%`-variable prefix of `text`, including the `%`.
fn scan_soft_var(text: &str) -> Option<usize> {
    let mut end = 1usize;
    for (offset, character) in text[1..].char_indices() {
        if character.is_ascii_alphanumeric() || matches!(character, '[' | ']' | '.' | '%') {
            end = 1 + offset + character.len_utf8();
        } else {
            break;
        }
    }
    for candidate in (2..=end).rev() {
        if text[..candidate].parse::<VarRef>().is_ok() {
            return Some(candidate);
        }
    }
    None
}
