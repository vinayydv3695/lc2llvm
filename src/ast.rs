#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Var(String),
    Lam(String, Box<Expr>),
    App(Box<Expr>, Box<Expr>),
    Int(i64),
    Prim(String),
}
