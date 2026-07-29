use std::io::{Read, Write};
use std::process::ExitCode;

use clap::Parser;
use jphfmt::{DEFAULT_WIDTH, format_with_width};

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

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Rewrite each named file in place when formatting changes it
    #[arg(short = 'i', long, conflicts_with = "check", requires = "files")]
    in_place: bool,

    /// Exit non-zero if any input is not already formatted; write nothing
    #[arg(long)]
    check: bool,

    /// Column limit; tab width is 4
    #[arg(long, default_value_t = DEFAULT_WIDTH, value_name = "N")]
    width: usize,

    /// Files to format; none reads stdin
    #[arg(value_name = "FILE")]
    files: Vec<String>,
}

impl Args {
    /// `--check` and `-i` cannot both be set, so their order here decides nothing.
    fn mode(&self) -> Mode {
        match (self.in_place, self.check) {
            (true, _) => Mode::InPlace,
            (false, true) => Mode::Check,
            (false, false) => Mode::Stdout,
        }
    }
}

/// Returns `true` if any input differed from its formatted form.
fn run(args: &Args) -> std::io::Result<bool> {
    let mode = args.mode();
    if args.files.is_empty() {
        let src = {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        };
        let out = format_with_width(&src, args.width);
        if mode != Mode::Check {
            std::io::stdout().write_all(out.as_bytes())?;
        }
        return Ok(out != src);
    }
    let mut any_changed = false;
    for path in &args.files {
        let src = std::fs::read_to_string(path)?;
        let out = format_with_width(&src, args.width);
        let changed = out != src;
        any_changed |= changed;
        match mode {
            Mode::Stdout => std::io::stdout().write_all(out.as_bytes())?,
            Mode::InPlace if changed => std::fs::write(path, out)?,
            Mode::InPlace | Mode::Check => {}
        }
    }
    Ok(any_changed)
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(changed) if args.mode() == Mode::Check && changed => ExitCode::FAILURE,
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("jphfmt: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, Mode};
    use clap::Parser;
    use jphfmt::DEFAULT_WIDTH;

    fn parsed(argv: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("jphfmt").chain(argv.iter().copied()))
    }

    #[test]
    fn no_arguments_formats_stdin_at_the_default_width() {
        let args = parsed(&[]).unwrap();
        assert_eq!(args.mode(), Mode::Stdout);
        assert_eq!(args.width, DEFAULT_WIDTH);
        assert!(args.files.is_empty());
    }

    #[test]
    fn width_takes_a_value_either_way() {
        assert_eq!(parsed(&["--width", "80"]).unwrap().width, 80);
        assert_eq!(parsed(&["--width=80"]).unwrap().width, 80);
        assert!(parsed(&["--width"]).is_err());
        assert!(parsed(&["--width", "wide"]).is_err());
    }

    #[test]
    fn in_place_requires_a_file() {
        assert!(parsed(&["-i"]).is_err());
        assert_eq!(parsed(&["-i", "a.c"]).unwrap().mode(), Mode::InPlace);
        assert_eq!(
            parsed(&["--in-place", "a.c"]).unwrap().mode(),
            Mode::InPlace
        );
    }

    #[test]
    fn in_place_and_check_are_exclusive() {
        assert!(parsed(&["-i", "--check", "a.c"]).is_err());
        assert_eq!(parsed(&["--check", "a.c"]).unwrap().mode(), Mode::Check);
    }

    /// A lone `-` is a file name rather than a flag, which the hand-rolled parser special-cased.
    #[test]
    fn a_lone_hyphen_is_a_file() {
        assert_eq!(parsed(&["-"]).unwrap().files, ["-"]);
        assert!(parsed(&["-x"]).is_err());
    }

    #[test]
    fn every_file_is_kept_in_order() {
        assert_eq!(
            parsed(&["b.c", "a.c", "b.c"]).unwrap().files,
            ["b.c", "a.c", "b.c"]
        );
    }
}
