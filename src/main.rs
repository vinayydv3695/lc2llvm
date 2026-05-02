mod ast;
mod cli;
mod codegen;
mod interpreter;
mod lexer;
mod parser;
mod transform;

fn main() {
    if let Err(err) = cli::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
