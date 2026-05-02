use std::collections::{HashMap, HashSet};

use crate::ast::Expr;

#[derive(Clone, Debug)]
pub struct PipelineOutput {
    pub alpha: Expr,
    pub closure: CCExpr,
    pub lifted: LiftedProgram,
    pub anf: AnfProgram,
}

pub fn run_pipeline(expr: &Expr) -> PipelineOutput {
    let alpha = alpha_convert(expr);
    let closure = closure_convert(&alpha);
    let lifted = lambda_lift(&closure);
    let anf = anf_transform(&lifted);
    PipelineOutput {
        alpha,
        closure,
        lifted,
        anf,
    }
}

pub fn alpha_convert(expr: &Expr) -> Expr {
    let mut gen = NameGen::new("a");
    let mut env = HashMap::new();
    alpha_expr(expr, &mut env, &mut gen)
}

fn alpha_expr(expr: &Expr, env: &mut HashMap<String, String>, gen: &mut NameGen) -> Expr {
    match expr {
        Expr::Var(v) => Expr::Var(env.get(v).cloned().unwrap_or_else(|| v.clone())),
        Expr::Int(n) => Expr::Int(*n),
        Expr::Prim(p) => Expr::Prim(p.clone()),
        Expr::App(f, a) => Expr::App(
            Box::new(alpha_expr(f, env, gen)),
            Box::new(alpha_expr(a, env, gen)),
        ),
        Expr::Lam(param, body) => {
            let fresh = gen.fresh(param);
            let prev = env.insert(param.clone(), fresh.clone());
            let new_body = alpha_expr(body, env, gen);
            if let Some(old) = prev {
                env.insert(param.clone(), old);
            } else {
                env.remove(param);
            }
            Expr::Lam(fresh, Box::new(new_body))
        }
    }
}

pub fn free_vars(expr: &Expr) -> HashSet<String> {
    match expr {
        Expr::Var(v) => HashSet::from([v.clone()]),
        Expr::Int(_) => HashSet::new(),
        Expr::Prim(_) => HashSet::new(),
        Expr::App(f, a) => {
            let mut out = free_vars(f);
            out.extend(free_vars(a));
            out
        }
        Expr::Lam(param, body) => {
            let mut out = free_vars(body);
            out.remove(param);
            out
        }
    }
}

#[derive(Clone, Debug)]
pub enum CCExpr {
    Var(String),
    LamClosure {
        param: String,
        free_vars: Vec<String>,
        body: Box<CCExpr>,
    },
    App(Box<CCExpr>, Box<CCExpr>),
    Int(i64),
    Prim(String),
}

pub fn closure_convert(expr: &Expr) -> CCExpr {
    match expr {
        Expr::Var(v) => CCExpr::Var(v.clone()),
        Expr::Int(n) => CCExpr::Int(*n),
        Expr::Prim(p) => CCExpr::Prim(p.clone()),
        Expr::App(f, a) => CCExpr::App(Box::new(closure_convert(f)), Box::new(closure_convert(a))),
        Expr::Lam(param, body) => {
            let mut fv = free_vars(body);
            fv.remove(param);
            let mut free_vars = fv.into_iter().collect::<Vec<_>>();
            free_vars.sort();
            CCExpr::LamClosure {
                param: param.clone(),
                free_vars,
                body: Box::new(closure_convert(body)),
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct LiftedProgram {
    pub functions: Vec<LiftedFunction>,
    pub main: LiftedExpr,
}

#[derive(Clone, Debug)]
pub struct LiftedFunction {
    pub name: String,
    pub param: String,
    pub free_vars: Vec<String>,
    pub body: LiftedExpr,
}

#[derive(Clone, Debug)]
pub enum LiftedExpr {
    Var(String),
    App(Box<LiftedExpr>, Box<LiftedExpr>),
    Int(i64),
    Prim(String),
    MakeClosure { func: String, captures: Vec<String> },
}

pub fn lambda_lift(expr: &CCExpr) -> LiftedProgram {
    let mut functions = Vec::new();
    let mut gen = NameGen::new("lam");
    let main = lift_expr(expr, &mut functions, &mut gen);
    LiftedProgram { functions, main }
}

fn lift_expr(expr: &CCExpr, functions: &mut Vec<LiftedFunction>, gen: &mut NameGen) -> LiftedExpr {
    match expr {
        CCExpr::Var(v) => LiftedExpr::Var(v.clone()),
        CCExpr::Int(n) => LiftedExpr::Int(*n),
        CCExpr::Prim(p) => LiftedExpr::Prim(p.clone()),
        CCExpr::App(f, a) => LiftedExpr::App(
            Box::new(lift_expr(f, functions, gen)),
            Box::new(lift_expr(a, functions, gen)),
        ),
        CCExpr::LamClosure {
            param,
            free_vars,
            body,
        } => {
            let name = gen.fresh("f");
            let lifted_body = lift_expr(body, functions, gen);
            functions.push(LiftedFunction {
                name: name.clone(),
                param: param.clone(),
                free_vars: free_vars.clone(),
                body: lifted_body,
            });
            LiftedExpr::MakeClosure {
                func: name,
                captures: free_vars.clone(),
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnfProgram {
    pub functions: Vec<AnfFunction>,
    pub main: AnfExpr,
}

#[derive(Clone, Debug)]
pub struct AnfFunction {
    pub name: String,
    pub param: String,
    pub free_vars: Vec<String>,
    pub body: AnfExpr,
}

#[derive(Clone, Debug)]
pub enum AnfExpr {
    Let(String, AnfRhs, Box<AnfExpr>),
    Return(AnfAtom),
}

#[derive(Clone, Debug)]
pub enum AnfRhs {
    App(AnfAtom, AnfAtom),
}

#[derive(Clone, Debug)]
pub enum AnfAtom {
    Var(String),
    Int(i64),
    Prim(String),
    MakeClosure { func: String, captures: Vec<String> },
}

pub fn anf_transform(program: &LiftedProgram) -> AnfProgram {
    let mut gen = NameGen::new("t");
    let functions = program
        .functions
        .iter()
        .map(|f| AnfFunction {
            name: f.name.clone(),
            param: f.param.clone(),
            free_vars: f.free_vars.clone(),
            body: lower_to_anf(&f.body, &mut gen),
        })
        .collect();

    let main = lower_to_anf(&program.main, &mut gen);

    AnfProgram { functions, main }
}

fn lower_to_anf(expr: &LiftedExpr, gen: &mut NameGen) -> AnfExpr {
    let (bindings, atom) = lower_expr(expr, gen);
    assemble(bindings, atom)
}

fn assemble(bindings: Vec<(String, AnfRhs)>, atom: AnfAtom) -> AnfExpr {
    let mut out = AnfExpr::Return(atom);
    for (name, rhs) in bindings.into_iter().rev() {
        out = AnfExpr::Let(name, rhs, Box::new(out));
    }
    out
}

fn lower_expr(expr: &LiftedExpr, gen: &mut NameGen) -> (Vec<(String, AnfRhs)>, AnfAtom) {
    match expr {
        LiftedExpr::Var(v) => (Vec::new(), AnfAtom::Var(v.clone())),
        LiftedExpr::Int(n) => (Vec::new(), AnfAtom::Int(*n)),
        LiftedExpr::Prim(p) => (Vec::new(), AnfAtom::Prim(p.clone())),
        LiftedExpr::MakeClosure { func, captures } => (
            Vec::new(),
            AnfAtom::MakeClosure {
                func: func.clone(),
                captures: captures.clone(),
            },
        ),
        LiftedExpr::App(f, a) => {
            let (mut b1, af) = lower_expr(f, gen);
            let (mut b2, aa) = lower_expr(a, gen);
            let tmp = gen.fresh("v");
            let mut out = Vec::new();
            out.append(&mut b1);
            out.append(&mut b2);
            out.push((tmp.clone(), AnfRhs::App(af, aa)));
            (out, AnfAtom::Var(tmp))
        }
    }
}

struct NameGen {
    prefix: String,
    next_id: usize,
}

impl NameGen {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            next_id: 0,
        }
    }

    fn fresh(&mut self, stem: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("{}_{}_{}", self.prefix, stem, id)
    }
}
