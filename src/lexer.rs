#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Lambda,
    Dot,
    LParen,
    RParen,
    Ident(String),
    Int(i64),
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();

    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        match ch {
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            '.' => {
                chars.next();
                tokens.push(Token::Dot);
            }
            '\\' | 'λ' => {
                chars.next();
                tokens.push(Token::Lambda);
            }
            '0'..='9' => {
                let mut n = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        n.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let v = n.parse::<i64>().map_err(|e| e.to_string())?;
                tokens.push(Token::Int(v));
            }
            _ => {
                if is_ident_start(ch) {
                    let mut id = String::new();
                    while let Some(&c) = chars.peek() {
                        if is_ident_continue(c) {
                            id.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::Ident(id));
                } else {
                    return Err(format!("unexpected character: {ch}"));
                }
            }
        }
    }

    Ok(tokens)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}
