// SPDX-License-Identifier: GPL-3.0-or-later

//! gettext `Plural-Forms` expressions: the C subset gettext uses (`n`,
//! numbers, `! * / % + - < <= > >= == != && || ?:` and parentheses),
//! parsed once and evaluated per count.

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    N,
    Num(u64),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl Expr {
    /// The plural form index for `n`.
    pub fn eval(&self, n: u64) -> u64 {
        match self {
            Self::N => n,
            Self::Num(v) => *v,
            Self::Not(e) => u64::from(e.eval(n) == 0),
            Self::If(c, a, b) => {
                if c.eval(n) != 0 {
                    a.eval(n)
                } else {
                    b.eval(n)
                }
            }
            Self::Bin(op, a, b) => {
                let (x, y) = (a.eval(n), b.eval(n));
                match op {
                    Op::Mul => x.wrapping_mul(y),
                    Op::Div => x.checked_div(y).unwrap_or(0),
                    Op::Rem => x.checked_rem(y).unwrap_or(0),
                    Op::Add => x.wrapping_add(y),
                    Op::Sub => x.wrapping_sub(y),
                    Op::Lt => u64::from(x < y),
                    Op::Le => u64::from(x <= y),
                    Op::Gt => u64::from(x > y),
                    Op::Ge => u64::from(x >= y),
                    Op::Eq => u64::from(x == y),
                    Op::Ne => u64::from(x != y),
                    Op::And => u64::from(x != 0 && y != 0),
                    Op::Or => u64::from(x != 0 || y != 0),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    N,
    Num(u64),
    Op(&'static str),
    Open,
    Close,
    Quest,
    Colon,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    const OPS: [&str; 15] = [
        "<=", ">=", "==", "!=", "&&", "||", "<", ">", "!", "*", "/", "%", "+", "-", "=",
    ];
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap_or(' ');
        if c.is_ascii_digit() {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            out.push(Tok::Num(rest[..end].parse().map_err(|e| format!("{e}"))?));
            rest = &rest[end..];
        } else if c == 'n' {
            out.push(Tok::N);
            rest = &rest[1..];
        } else if c == '(' || c == ')' || c == '?' || c == ':' {
            out.push(match c {
                '(' => Tok::Open,
                ')' => Tok::Close,
                '?' => Tok::Quest,
                _ => Tok::Colon,
            });
            rest = &rest[1..];
        } else if let Some(op) = OPS.iter().find(|o| rest.starts_with(**o)) {
            if *op == "=" {
                return Err("single '=' in plural expression".into());
            }
            out.push(Tok::Op(op));
            rest = &rest[op.len()..];
        } else {
            return Err(format!("unexpected '{c}' in plural expression"));
        }
        rest = rest.trim_start();
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    /// `cond ? a : b` (right associative).
    fn ternary(&mut self) -> Result<Expr, String> {
        let c = self.binary(0)?;
        if self.peek() == Some(&Tok::Quest) {
            self.pos += 1;
            let a = self.ternary()?;
            if self.next() != Some(Tok::Colon) {
                return Err("missing ':' in plural expression".into());
            }
            let b = self.ternary()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    /// Binary operators by precedence level, lowest first.
    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[(&str, Op)]; 6] = [
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[("==", Op::Eq), ("!=", Op::Ne)],
            &[("<", Op::Lt), ("<=", Op::Le), (">", Op::Gt), (">=", Op::Ge)],
            &[("+", Op::Add), ("-", Op::Sub)],
            &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
        ];
        let Some(ops) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut left = self.binary(level + 1)?;
        while let Some(Tok::Op(s)) = self.peek() {
            let Some((_, op)) = ops.iter().find(|(o, _)| o == s) else {
                break;
            };
            self.pos += 1;
            let right = self.binary(level + 1)?;
            left = Expr::Bin(*op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Op("!")) => Ok(Expr::Not(Box::new(self.unary()?))),
            Some(Tok::N) => Ok(Expr::N),
            Some(Tok::Num(v)) => Ok(Expr::Num(v)),
            Some(Tok::Open) => {
                let e = self.ternary()?;
                if self.next() != Some(Tok::Close) {
                    return Err("missing ')' in plural expression".into());
                }
                Ok(e)
            }
            other => Err(format!("unexpected {other:?} in plural expression")),
        }
    }
}

/// Parses an expression such as `(n != 1)`.
pub fn parse(s: &str) -> Result<Expr, String> {
    let mut p = Parser {
        toks: lex(s)?,
        pos: 0,
    };
    let e = p.ternary()?;
    if p.pos != p.toks.len() {
        return Err("trailing tokens in plural expression".into());
    }
    Ok(e)
}

/// `nplurals` and the expression from a `Plural-Forms` header value
/// (`nplurals=2; plural=(n != 1);`).
pub fn from_header(value: &str) -> Result<(usize, Expr), String> {
    let mut nplurals = None;
    let mut plural = None;
    for part in value.split(';') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.trim() {
            "nplurals" => nplurals = v.trim().parse::<usize>().ok(),
            "plural" => plural = Some(parse(v)?),
            _ => {}
        }
    }
    match (nplurals, plural) {
        (Some(n), Some(e)) if n > 0 => Ok((n, e)),
        _ => Err(format!("bad Plural-Forms: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(header: &str, ns: &[u64]) -> Vec<u64> {
        let (_, e) = from_header(header).expect("header");
        ns.iter().map(|n| e.eval(*n)).collect()
    }

    #[test]
    fn common_plural_rules_pick_the_right_form() {
        assert_eq!(forms("nplurals=2; plural=(n != 1);", &[0, 1, 2]), [1, 0, 1]);
        assert_eq!(forms("nplurals=1; plural=0;", &[0, 1, 7]), [0, 0, 0]);
        // Russian: one, few, many.
        let ru = "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);";
        assert_eq!(
            forms(ru, &[1, 21, 101, 2, 4, 22, 102, 0, 5, 11, 12, 25, 111]),
            [0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]
        );
        // Arabic: six forms.
        let ar = "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);";
        assert_eq!(
            forms(ar, &[0, 1, 2, 3, 10, 11, 99, 100, 102]),
            [0, 1, 2, 3, 3, 4, 4, 5, 5]
        );
        // French: 0 and 1 are singular.
        assert_eq!(forms("nplurals=2; plural=(n > 1);", &[0, 1, 2]), [0, 0, 1]);
        assert_eq!(forms("nplurals=2; plural=!(n == 1);", &[1, 3]), [0, 1]);
    }

    #[test]
    fn broken_expressions_are_refused() {
        for bad in [
            "n = 1",
            "(n != 1",
            "n ?",
            "n % ",
            "x",
            "nplurals=0; plural=0;",
        ] {
            assert!(
                parse(bad).is_err() || from_header(bad).is_err(),
                "{bad} accepted"
            );
        }
        assert!(from_header("nplurals=2;").is_err());
    }
}
