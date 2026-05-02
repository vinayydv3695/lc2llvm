use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codegen::generate_llvm_ir;
use crate::lexer::tokenize;
use crate::parser::parse;
use crate::transform::run_pipeline;

pub fn run() -> Result<(), String> {
    let (input, output) = parse_args(env::args().collect())?;
    let source = fs::read_to_string(&input)
        .map_err(|e| format!("failed to read {}: {e}", input.display()))?;

    let tokens = tokenize(&source)?;
    let expr = parse(&tokens)?;
    let pipeline = run_pipeline(&expr);
    let _ = (&pipeline.alpha, &pipeline.closure, &pipeline.lifted);
    let ir = generate_llvm_ir(&pipeline.anf)?;

    match infer_output_kind(&output) {
        OutputKind::Ll => {
            fs::write(&output, ir)
                .map_err(|e| format!("failed to write {}: {e}", output.display()))?;
        }
        OutputKind::Obj => emit_object(&ir, &output)?,
        OutputKind::Exe => emit_executable(&ir, &output)?,
    }

    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<(PathBuf, PathBuf), String> {
    if args.len() < 2 {
        return Err("usage: lamc <input.lc> [-o output]".to_string());
    }

    let mut input: Option<PathBuf> = None;
    let mut output = PathBuf::from("output.ll");
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        if arg == "-o" {
            if i + 1 >= args.len() {
                return Err("-o requires output path".to_string());
            }
            output = PathBuf::from(&args[i + 1]);
            i += 2;
            continue;
        }

        if input.is_none() {
            input = Some(PathBuf::from(arg));
            i += 1;
            continue;
        }

        return Err(format!("unexpected argument: {arg}"));
    }

    let input = input.ok_or_else(|| "missing input path".to_string())?;
    Ok((input, output))
}

enum OutputKind {
    Ll,
    Obj,
    Exe,
}

fn infer_output_kind(output: &Path) -> OutputKind {
    let ext = output
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase());

    match ext.as_deref() {
        Some("ll") => OutputKind::Ll,
        Some("o") => OutputKind::Obj,
        _ => OutputKind::Exe,
    }
}

fn emit_object(ir: &str, output: &Path) -> Result<(), String> {
    let ll_path = temp_path("lamc", "ll");
    fs::write(&ll_path, ir)
        .map_err(|e| format!("failed to write temporary {}: {e}", ll_path.display()))?;

    let res = run_clang(&[
        "-c".to_string(),
        path_arg(&ll_path),
        "-o".to_string(),
        path_arg(output),
    ]);

    cleanup_files(&[ll_path]);
    res
}

fn emit_executable(ir: &str, output: &Path) -> Result<(), String> {
    let ll_path = temp_path("lamc", "ll");
    let obj_path = temp_path("lamc", "o");
    let runtime_c_path = temp_path("lamc_runtime", "c");
    let runtime_obj_path = temp_path("lamc_runtime", "o");

    fs::write(&ll_path, ir)
        .map_err(|e| format!("failed to write temporary {}: {e}", ll_path.display()))?;
    fs::write(&runtime_c_path, include_str!("../runtime.c")).map_err(|e| {
        format!(
            "failed to write temporary {}: {e}",
            runtime_c_path.display()
        )
    })?;

    let compile_ir_res = run_clang(&[
        "-c".to_string(),
        path_arg(&ll_path),
        "-o".to_string(),
        path_arg(&obj_path),
    ]);
    if let Err(err) = compile_ir_res {
        cleanup_files(&[ll_path, runtime_c_path, obj_path, runtime_obj_path]);
        return Err(err);
    }

    let compile_runtime_res = run_clang(&[
        "-c".to_string(),
        path_arg(&runtime_c_path),
        "-o".to_string(),
        path_arg(&runtime_obj_path),
    ]);
    if let Err(err) = compile_runtime_res {
        cleanup_files(&[ll_path, runtime_c_path, obj_path, runtime_obj_path]);
        return Err(err);
    }

    let link_res = run_clang(&[
        path_arg(&obj_path),
        path_arg(&runtime_obj_path),
        "-o".to_string(),
        path_arg(output),
    ]);

    cleanup_files(&[ll_path, runtime_c_path, obj_path, runtime_obj_path]);
    link_res
}

fn run_clang(args: &[String]) -> Result<(), String> {
    let output = Command::new("clang")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run clang: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("clang failed with status {}", output.status))
    } else {
        Err(format!("clang failed: {stderr}"))
    }
}

fn cleanup_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn temp_path(prefix: &str, extension: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    env::temp_dir().join(format!(
        "{prefix}_{}_{}.{}",
        process::id(),
        stamp,
        extension
    ))
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
