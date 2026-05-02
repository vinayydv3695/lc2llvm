use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::Expr;

#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Closure { param: String, body: Expr, env: Env },
    Prim(PrimValue),
}

#[derive(Clone, Debug)]
pub enum PrimValue {
    Print,
    Add(Option<i64>),
    Sub(Option<i64>),
    Mul(Option<i64>),
    If(IfState),
}

#[derive(Clone, Debug)]
pub enum IfState {
    AwaitCond,
    AwaitThen { cond: bool },
    AwaitElse { cond: bool, then_branch: Thunk },
}

#[derive(Clone, Debug)]
pub struct Thunk {
    expr: Expr,
    env: Env,
}

#[derive(Clone, Debug)]
pub struct Env(Rc<EnvFrame>);

#[derive(Clone, Debug)]
struct EnvFrame {
    parent: Option<Env>,
    bindings: HashMap<String, Value>,
}

pub fn eval(expr: &Expr) -> Result<Value, String> {
    let env = Env::empty();
    eval_expr(expr, &env)
}

pub fn format_value(value: &Value) -> String {
    match value {
        Value::Int(n) => n.to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Closure { .. } => "<closure>".to_string(),
        Value::Prim(_) => "<prim>".to_string(),
    }
}

fn eval_expr(expr: &Expr, env: &Env) -> Result<Value, String> {
    match expr {
        Expr::Var(name) => env.lookup(name),
        Expr::Lam(param, body) => Ok(Value::Closure {
            param: param.clone(),
            body: (**body).clone(),
            env: env.clone(),
        }),
        Expr::App(func, arg) => {
            let func = eval_expr(func, env)?;
            apply(func, arg, env)
        }
        Expr::Int(n) => Ok(Value::Int(*n)),
        Expr::Prim(name) => Ok(match name.as_str() {
            "print" => Value::Prim(PrimValue::Print),
            "add" => Value::Prim(PrimValue::Add(None)),
            "sub" => Value::Prim(PrimValue::Sub(None)),
            "mul" => Value::Prim(PrimValue::Mul(None)),
            "if" => Value::Prim(PrimValue::If(IfState::AwaitCond)),
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(format!("unknown primitive: {name}")),
        }),
    }
}

fn apply(func: Value, arg: &Expr, env: &Env) -> Result<Value, String> {
    match func {
        Value::Closure {
            param,
            body,
            env: closure_env,
        } => {
            let arg_value = eval_expr(arg, env)?;
            let next_env = closure_env.extend(param, arg_value);
            eval_expr(&body, &next_env)
        }
        Value::Prim(prim) => apply_prim(prim, arg, env),
        _ => Err("attempted to apply a non-function value".to_string()),
    }
}

fn apply_prim(prim: PrimValue, arg: &Expr, env: &Env) -> Result<Value, String> {
    match prim {
        PrimValue::Print => {
            let value = eval_expr(arg, env)?;
            let n = expect_int(&value)?;
            println!("{n}");
            Ok(Value::Int(n))
        }
        PrimValue::Add(None) => {
            let lhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Prim(PrimValue::Add(Some(lhs))))
        }
        PrimValue::Add(Some(lhs)) => {
            let rhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Int(lhs + rhs))
        }
        PrimValue::Sub(None) => {
            let lhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Prim(PrimValue::Sub(Some(lhs))))
        }
        PrimValue::Sub(Some(lhs)) => {
            let rhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Int(lhs - rhs))
        }
        PrimValue::Mul(None) => {
            let lhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Prim(PrimValue::Mul(Some(lhs))))
        }
        PrimValue::Mul(Some(lhs)) => {
            let rhs = expect_int(&eval_expr(arg, env)?)?;
            Ok(Value::Int(lhs * rhs))
        }
        PrimValue::If(IfState::AwaitCond) => {
            let cond = expect_bool(&eval_expr(arg, env)?)?;
            Ok(Value::Prim(PrimValue::If(IfState::AwaitThen { cond })))
        }
        PrimValue::If(IfState::AwaitThen { cond }) => {
            Ok(Value::Prim(PrimValue::If(IfState::AwaitElse {
                cond,
                then_branch: Thunk::new(arg, env),
            })))
        }
        PrimValue::If(IfState::AwaitElse { cond, then_branch }) => {
            if cond {
                eval_thunk(&then_branch)
            } else {
                eval_expr(arg, env)
            }
        }
    }
}

fn eval_thunk(thunk: &Thunk) -> Result<Value, String> {
    eval_expr(&thunk.expr, &thunk.env)
}

fn expect_int(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(*n),
        _ => Err("expected integer value".to_string()),
    }
}

fn expect_bool(value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(v) => Ok(*v),
        _ => Err("expected boolean value".to_string()),
    }
}

impl Env {
    fn empty() -> Self {
        Self(Rc::new(EnvFrame {
            parent: None,
            bindings: HashMap::new(),
        }))
    }

    fn extend(&self, name: String, value: Value) -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(name, value);
        Self(Rc::new(EnvFrame {
            parent: Some(self.clone()),
            bindings,
        }))
    }

    fn lookup(&self, name: &str) -> Result<Value, String> {
        if let Some(value) = self.0.bindings.get(name) {
            return Ok(value.clone());
        }
        match &self.0.parent {
            Some(parent) => parent.lookup(name),
            None => Err(format!("unbound variable: {name}")),
        }
    }
}

impl Thunk {
    fn new(expr: &Expr, env: &Env) -> Self {
        Self {
            expr: expr.clone(),
            env: env.clone(),
        }
    }
}
