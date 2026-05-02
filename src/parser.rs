use crate::ast::Expr;
use crate::lexer::Token;

pub fn parse(tokens: &[Token]) -> Result<Expr, String> {
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_expr()?;
    if p.pos != p.tokens.len() {
        return Err("unexpected trailing tokens".to_string());
    }
    Ok(expr)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_app()
    }

    fn parse_app(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_atom()?;
        while self.can_start_atom() {
            let arg = self.parse_atom()?;
            expr = Expr::App(Box::new(expr), Box::new(arg));
        }
        Ok(expr)
    }

    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Lambda) => self.parse_lam(),
            Some(Token::LParen) => {
                self.bump();
                let expr = self.parse_expr()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(expr),
                    _ => Err("expected ')'".to_string()),
                }
            }
            Some(Token::Ident(name)) => {
                let n = name.clone();
                self.bump();
                if is_primitive(&n) {
                    Ok(Expr::Prim(n))
                } else {
                    Ok(Expr::Var(n))
                }
            }
            Some(Token::Int(v)) => {
                let n = *v;
                self.bump();
                Ok(Expr::Int(n))
            }
            _ => Err("expected expression".to_string()),
        }
    }

    fn parse_lam(&mut self) -> Result<Expr, String> {
        self.bump();
        let param = match self.bump() {
            Some(Token::Ident(name)) => name.clone(),
            _ => return Err("expected parameter after lambda".to_string()),
        };
        match self.bump() {
            Some(Token::Dot) => {}
            _ => return Err("expected '.' after lambda parameter".to_string()),
        }
        let body = self.parse_expr()?;
        Ok(Expr::Lam(param, Box::new(body)))
    }

    fn can_start_atom(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::Lambda) | Some(Token::LParen) | Some(Token::Ident(_)) | Some(Token::Int(_))
        )
    }

    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }
}

fn is_primitive(name: &str) -> bool {
    matches!(name, "print")
}
