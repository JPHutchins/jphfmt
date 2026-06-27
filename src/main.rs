use std::io::{Read, Write};
use std::process::ExitCode;

use cfmt::format;

/// What to do with each input's formatted result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Format stdin or the named files to stdout.
    Stdout,
    /// Rewrite each named file in place when formatting changes it.
    InPlace,
    /// Report (via exit code) whether any input is not already formatted; write nothing.
    Check,
}

struct Args {
    mode: Mode,
    files: Vec<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut mode = Mode::Stdout;
    let mut files = Vec::new();
    for arg in argv {
        match arg.as_str() {
            "-i" | "--in-place" => mode = Mode::InPlace,
            "--check" => mode = Mode::Check,
            "-h" | "--help" => return Err("usage: cfmt [-i | --check] [FILE...]".to_owned()),
            flag if flag.starts_with('-') && flag != "-" => {
                return Err(format!("unknown flag: {flag}"));
            }
            _ => files.push(arg.clone()),
        }
    }
    if mode == Mode::InPlace && files.is_empty() {
        return Err("-i requires at least one FILE".to_owned());
    }
    Ok(Args { mode, files })
}

fn read_stdin() -> std::io::Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

/// Returns `true` if any input differed from its formatted form.
fn run(args: &Args) -> std::io::Result<bool> {
    if args.files.is_empty() {
        let src = read_stdin()?;
        let out = format(&src);
        if args.mode != Mode::Check {
            std::io::stdout().write_all(out.as_bytes())?;
        }
        return Ok(out != src);
    }
    let mut any_changed = false;
    for path in &args.files {
        let src = std::fs::read_to_string(path)?;
        let out = format(&src);
        let changed = out != src;
        any_changed |= changed;
        match args.mode {
            Mode::Stdout => std::io::stdout().write_all(out.as_bytes())?,
            Mode::InPlace if changed => std::fs::write(path, out)?,
            Mode::InPlace | Mode::Check => {}
        }
    }
    Ok(any_changed)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("cfmt: {msg}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(changed) if args.mode == Mode::Check && changed => ExitCode::FAILURE,
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cfmt: {err}");
            ExitCode::FAILURE
        }
    }
}
