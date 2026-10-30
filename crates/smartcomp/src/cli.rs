use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};

use crate::archive::{self, CreateOptions};
use crate::codecs::Level;
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LevelArg {
    Store,
    Medium,
    Ultra,
    /// Maximum ratio (nanozip context mixing): smallest archives, several times slower than ultra
    #[value(name = "highultra")]
    HighUltra,
}

impl From<LevelArg> for Level {
    fn from(l: LevelArg) -> Level {
        match l {
            LevelArg::Store => Level::Store,
            LevelArg::Medium => Level::Medium,
            LevelArg::Ultra => Level::Ultra,
            LevelArg::HighUltra => Level::HighUltra,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "smartcomp",
    version = "0.1",
    about = "SmartCompression v 0.1 - smart per-file archiver: detects each file's format and picks the compressor that suits it",
    after_help = "Levels:
  store      no compression
  medium     fast, similar size to 7-Zip Ultra
  ultra      strong coders compared on each file (PPMd, UHARC, LZMA2 + filters)
  highultra  maximum ratio with nanozip context mixing; several times slower than ultra

Examples:
  smartcomp -a photos.sca ~/Pictures --level ultra
  smartcomp -a backup.sca docs/ -P --level highultra -p 8   (prompts for a password)
  smartcomp -e photos.sca -o restored/
  smartcomp -l photos.sca

Project page: https://github.com/farhad-labs/SmartCompression
Free for non-commercial use; commercial use requires permission (see LICENSE)."
)]
pub struct Cli {
    /// Create ARCHIVE from the input files/directories
    #[arg(short = 'a', value_name = "ARCHIVE", conflicts_with_all = ["extract", "list"])]
    pub add: Option<PathBuf>,

    /// Extract ARCHIVE
    #[arg(short = 'e', value_name = "ARCHIVE", conflicts_with = "list")]
    pub extract: Option<PathBuf>,

    /// List the contents of ARCHIVE
    #[arg(short = 'l', value_name = "ARCHIVE")]
    pub list: Option<PathBuf>,

    /// Files and directories to add (with -a)
    pub inputs: Vec<PathBuf>,

    #[arg(long, value_enum, default_value = "medium")]
    pub level: LevelArg,

    /// Output directory for -e
    #[arg(short = 'o', long, default_value = ".")]
    pub out: PathBuf,

    /// Password (AES-256). Use `-P` alone to be prompted, or `-P=SECRET`.
    #[arg(short = 'P', long, num_args = 0..=1, require_equals = true, default_missing_value = "")]
    pub password: Option<String>,

    /// Number of processor cores to use for archiving and extracting (default: all)
    #[arg(short = 'p', long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub threads: Option<u16>,

    /// Print one line per file with the chosen codec
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// No progress bar and no summary (errors still print)
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Force one codec for every file (developer option)
    #[arg(long, hide = true)]
    pub codec: Option<String>,
}

fn resolve_password(arg: &Option<String>, confirm: bool) -> Result<Option<String>> {
    match arg.as_deref() {
        None => Ok(None),
        Some("") => {
            let pw = rpassword::prompt_password("Password: ")?;
            if confirm && rpassword::prompt_password("Repeat password: ")? != pw {
                bail!("passwords do not match");
            }
            if pw.is_empty() {
                bail!("empty password");
            }
            Ok(Some(pw))
        }
        Some(pw) => Ok(Some(pw.to_string())),
    }
}

fn cores(cli: &Cli) -> usize {
    cli.threads.map(usize::from).unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

pub fn run(cli: Cli) -> Result<()> {
    let threads = cores(&cli);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().ok();

    if !cli.quiet {
        ui::print_banner();
    }
    let started = std::time::Instant::now();
    if let Some(archive_path) = &cli.add {
        if cli.inputs.is_empty() {
            bail!("-a needs at least one input file or directory");
        }
        let pw = resolve_password(&cli.password, true)?;
        let level: Level = cli.level.into();
        let opts = CreateOptions {
            level,
            password: pw.as_deref(),
            verbose: cli.verbose,
            force_codec: cli.codec.as_deref(),
            progress: !cli.quiet,
        };
        let m = archive::create(archive_path, &cli.inputs, &opts)?;
        if !cli.quiet {
            let size = std::fs::metadata(archive_path)?.len();
            ui::print_create_summary(&archive_path.display().to_string(), &m, size, started.elapsed(), threads, level.name());
        }
    } else if let Some(archive_path) = &cli.extract {
        let pw = resolve_password(&cli.password, false)?;
        let n = archive::extract(archive_path, &cli.out, pw.as_deref(), cli.verbose, !cli.quiet)?;
        if !cli.quiet {
            let (_, m) = archive::read_manifest(archive_path, pw.as_deref())?;
            let bytes: u64 = m.entries.iter().map(|e| e.original_size).sum();
            ui::print_extract_summary(n, bytes, &cli.out.display().to_string(), started.elapsed(), threads);
        }
    } else if let Some(archive_path) = &cli.list {
        let pw = resolve_password(&cli.password, false)?;
        let (_, m) = archive::read_manifest(archive_path, pw.as_deref())?;
        ui::print_listing(&m, std::fs::metadata(archive_path)?.len());
    } else {
        bail!("nothing to do: use -a, -e or -l (see --help)");
    }
    Ok(())
}
