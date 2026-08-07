use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

pub struct CalcTool;

#[derive(Deserialize)]
pub struct CalcArgs {
    expression: String,
}

#[async_trait]
impl TypedTool for CalcTool {
    type Args = CalcArgs;
    const NAME: &'static str = "calc";

    async fn run(
        &self,
        args: CalcArgs,
        responder: &dyn ToolCallResponder,
        _cancellation: CancellationToken,
    ) -> Result<(), MorayError> {
        let payload = match eval_expr(&args.expression) {
            Ok(value) => serde_json::json!({ "value": value }),
            Err(msg) => serde_json::json!({ "error": msg }),
        };
        let text = serde_json::to_string(&payload)
            .map_err(|e| MorayError::Message(format!("calc: serialization failed: {e}")))?;
        responder.send_text(text).await?;
        Ok(())
    }
}

fn eval_expr(input: &str) -> Result<f64, String> {
    let mut parser = Parser::new(input);
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.peek().is_some() {
        return Err("invalid expression".to_string());
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    value += self.parse_term()?;
                }
                Some('-') => {
                    self.pos += 1;
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    value *= self.parse_factor()?;
                }
                Some('/') => {
                    self.pos += 1;
                    let rhs = self.parse_factor()?;
                    if rhs == 0.0 {
                        return Err("division by zero".to_string());
                    }
                    value /= rhs;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, String> {
        self.skip_ws();
        match self.peek() {
            Some('+') => {
                self.pos += 1;
                self.parse_factor()
            }
            Some('-') => {
                self.pos += 1;
                Ok(-self.parse_factor()?)
            }
            Some('(') => {
                self.pos += 1;
                let value = self.parse_expr()?;
                self.skip_ws();
                if self.consume(')') {
                    Ok(value)
                } else {
                    Err("missing ')'".to_string())
                }
            }
            Some(ch) if ch.is_ascii_alphabetic() => self.parse_function_call(),
            Some(ch) if ch.is_ascii_digit() || ch == '.' => self.parse_number(),
            Some(ch) => Err(format!("parse error near '{ch}'")),
            None => Err("invalid expression".to_string()),
        }
    }

    fn parse_function_call(&mut self) -> Result<f64, String> {
        let name = self.parse_identifier();
        self.skip_ws();
        if !self.consume('(') {
            return Err(format!("unknown token '{name}'"));
        }

        let mut args = Vec::new();
        self.skip_ws();
        if !self.consume(')') {
            loop {
                args.push(self.parse_expr()?);
                self.skip_ws();
                if self.consume(',') {
                    continue;
                }
                if self.consume(')') {
                    break;
                }
                return Err("expected ',' or ')'".to_string());
            }
        }
        apply_function(&name, &args)
    }

    fn parse_identifier(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() || ch.is_ascii_digit() || ch == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let token: String = self.chars[start..self.pos].iter().collect();
        token
            .parse::<f64>()
            .map_err(|_| format!("parse error near '{token}'"))
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        match self.peek() {
            Some(ch) if ch == expected => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
}

fn apply_function(name: &str, args: &[f64]) -> Result<f64, String> {
    match name {
        "sqrt" => unary(name, args, |x| {
            if x < 0.0 {
                Err("sqrt domain error".to_string())
            } else {
                Ok(x.sqrt())
            }
        }),
        "abs" => unary(name, args, |x| Ok(x.abs())),
        "round" => unary(name, args, |x| Ok(x.round())),
        "floor" => unary(name, args, |x| Ok(x.floor())),
        "ceil" => unary(name, args, |x| Ok(x.ceil())),
        "sin" => unary(name, args, |x| Ok(x.sin())),
        "cos" => unary(name, args, |x| Ok(x.cos())),
        "tan" => unary(name, args, |x| Ok(x.tan())),
        "ln" => unary(name, args, |x| {
            if x <= 0.0 {
                Err("ln domain error".to_string())
            } else {
                Ok(x.ln())
            }
        }),
        "log10" => unary(name, args, |x| {
            if x <= 0.0 {
                Err("log10 domain error".to_string())
            } else {
                Ok(x.log10())
            }
        }),
        "exp" => unary(name, args, |x| Ok(x.exp())),
        "pow" => binary(name, args, |x, y| Ok(x.powf(y))),
        "max" => binary(name, args, |x, y| Ok(x.max(y))),
        "min" => binary(name, args, |x, y| Ok(x.min(y))),
        _ => Err(format!("unknown function '{name}'")),
    }
}

fn unary<F>(name: &str, args: &[f64], f: F) -> Result<f64, String>
where
    F: FnOnce(f64) -> Result<f64, String>,
{
    if args.len() != 1 {
        return Err(format!("{name} expects 1 argument"));
    }
    f(args[0])
}

fn binary<F>(name: &str, args: &[f64], f: F) -> Result<f64, String>
where
    F: FnOnce(f64, f64) -> Result<f64, String>,
{
    if args.len() != 2 {
        return Err(format!("{name} expects 2 arguments"));
    }
    f(args[0], args[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_operator_precedence() {
        let value = eval_expr("1 + 2 * 3").expect("expression should parse");
        assert_eq!(value, 7.0);
    }

    #[test]
    fn supports_parentheses() {
        let value = eval_expr("(1 + 2) * 3").expect("expression should parse");
        assert_eq!(value, 9.0);
    }

    #[test]
    fn supports_unary_operators() {
        let value = eval_expr("-2 + +5").expect("expression should parse");
        assert_eq!(value, 3.0);
    }

    #[test]
    fn supports_advanced_functions() {
        let value = eval_expr("pow(2, 3) + sqrt(9) + max(1, 4)").expect("expression should parse");
        assert_eq!(value, 15.0);
    }

    #[test]
    fn nested_function_calls_work() {
        let value = eval_expr("min(pow(3, 2), 10)").expect("expression should parse");
        assert_eq!(value, 9.0);
    }

    #[test]
    fn division_by_zero_returns_error() {
        let err = eval_expr("10 / 0").expect_err("should fail");
        assert!(err.contains("division by zero"), "{err}");
    }

    #[test]
    fn unknown_function_returns_error() {
        let err = eval_expr("foo(1)").expect_err("should fail");
        assert!(err.contains("unknown function"), "{err}");
    }

    #[test]
    fn parse_error_returns_error() {
        let err = eval_expr("1 + x").expect_err("should fail");
        assert!(
            err.contains("parse error") || err.contains("unknown token"),
            "{err}"
        );
    }
}
