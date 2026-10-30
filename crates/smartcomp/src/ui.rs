//! Terminal output: the live progress bar and the end-of-run summaries.
//!
//! The bar draws on stderr and hides itself when stderr is not a terminal (pipes, CI logs), so
//! scripted use gets clean output. Summaries go to stdout.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::manifest::Manifest;

pub const APP_NAME: &str = "SmartCompression v 0.1";
pub const REPO_URL: &str = "https://github.com/farhad-labs/SmartCompression";

/// Application name, version and project page, printed at the start of every run.
pub fn print_banner() {
    println!("{APP_NAME}  -  {REPO_URL}\n");
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn human_time(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        format!("{s:.1}s")
    } else {
        let t = s.round() as u64;
        if t < 3600 {
            format!("{}m {:02}s", t / 60, t % 60)
        } else {
            format!("{}h {:02}m {:02}s", t / 3600, (t % 3600) / 60, t % 60)
        }
    }
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        "-".into()
    } else {
        format!("{:.1}%", 100.0 * part as f64 / whole as f64)
    }
}

/// Live progress of a run over `total_files` files / `total_bytes` input bytes. All methods
/// take `&self` so worker threads can report concurrently.
pub struct Progress {
    bar: ProgressBar,
    state: Mutex<State>,
    total_files: u64,
}

#[derive(Default)]
struct State {
    active: Vec<String>,
    done: u64,
}

impl Progress {
    pub fn new(label: &str, total_bytes: u64, total_files: u64, enabled: bool) -> Progress {
        let bar = if enabled { ProgressBar::with_draw_target(Some(total_bytes), ProgressDrawTarget::stderr_with_hz(5)) } else { ProgressBar::hidden() };
        bar.set_style(
            ProgressStyle::with_template("{prefix:.bold} [{bar:34.cyan/blue}] {percent:>3}%  {bytes}/{total_bytes}  {binary_bytes_per_sec}  ETA {eta}\n  {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_prefix(label.to_string());
        bar.enable_steady_tick(Duration::from_millis(200));
        let p = Progress { bar, state: Mutex::new(State::default()), total_files };
        p.refresh_message();
        p
    }

    fn refresh_message(&self) {
        let s = self.state.lock().unwrap();
        let names: Vec<&str> = s.active.iter().take(4).map(|n| n.rsplit('/').next().unwrap_or(n)).collect();
        let more = s.active.len().saturating_sub(4);
        let working = if names.is_empty() {
            String::new()
        } else {
            format!("  working on: {}{}", names.join(", "), if more > 0 { format!(" +{more}") } else { String::new() })
        };
        self.bar.set_message(format!("files {}/{}{working}", s.done, self.total_files));
    }

    pub fn started(&self, name: &str) {
        self.state.lock().unwrap().active.push(name.to_string());
        self.refresh_message();
    }

    pub fn finished(&self, name: &str, bytes: u64) {
        {
            let mut s = self.state.lock().unwrap();
            if let Some(i) = s.active.iter().position(|n| n == name) {
                s.active.remove(i);
            }
            s.done += 1;
        }
        self.bar.inc(bytes);
        self.refresh_message();
    }

    /// Prints a line above the bar (or plainly on stderr when no bar is shown).
    pub fn line(&self, text: &str) {
        if self.bar.is_hidden() {
            eprintln!("{text}");
        } else {
            self.bar.println(text);
        }
    }

    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }
}

/// Totals and per-codec usage of a finished archive, for the summary after `-a`.
pub fn print_create_summary(archive: &str, manifest: &Manifest, archive_bytes: u64, elapsed: Duration, cores: usize, level: &str) {
    let original: u64 = manifest.entries.iter().map(|e| e.original_size).sum();
    println!("Archive   {archive}");
    println!("Level     {level}  ({cores} core{})", if cores == 1 { "" } else { "s" });
    println!(
        "Files     {}    {} -> {}   ({}, saved {})",
        manifest.entries.len(),
        human_bytes(original),
        human_bytes(archive_bytes),
        percent(archive_bytes, original),
        human_bytes(original.saturating_sub(archive_bytes))
    );
    let rate = original as f64 / elapsed.as_secs_f64().max(0.001);
    println!("Time      {}   ({}/s)", human_time(elapsed), human_bytes(rate as u64));

    let mut by_tool: BTreeMap<&str, (u64, u64, u64)> = BTreeMap::new();
    let mut fallbacks = 0;
    for e in &manifest.entries {
        let t = by_tool.entry(e.tool.as_str()).or_default();
        t.0 += 1;
        t.1 += e.original_size;
        t.2 += e.compressed_size;
        fallbacks += e.fallback_reason.is_some() as u32;
    }
    let mut rows: Vec<_> = by_tool.into_iter().collect();
    rows.sort_by_key(|(_, t)| std::cmp::Reverse(t.2));
    println!("\nCodecs");
    for (tool, (n, orig, packed)) in rows {
        println!("  {tool:<10} {n:>3} file{}  {:>10} -> {:>10}  ({})", if n == 1 { " " } else { "s" }, human_bytes(orig), human_bytes(packed), percent(packed, orig));
    }
    if fallbacks > 0 {
        println!("\n{fallbacks} file{} used a fallback (see fallback_reason in the manifest; -l shows it)", if fallbacks == 1 { "" } else { "s" });
    }
}

pub fn print_extract_summary(files: usize, bytes: u64, dest: &str, elapsed: Duration, cores: usize) {
    let rate = bytes as f64 / elapsed.as_secs_f64().max(0.001);
    println!(
        "Extracted {files} file{} ({}) to {dest} in {} ({}/s, {cores} core{})",
        if files == 1 { "" } else { "s" },
        human_bytes(bytes),
        human_time(elapsed),
        human_bytes(rate as u64),
        if cores == 1 { "" } else { "s" }
    );
}

pub fn print_listing(manifest: &Manifest, archive_bytes: u64) {
    println!("{:>3}  {:<9} {:<8} {:>11} {:>11} {:>7}  path", "id", "codec", "format", "original", "packed", "ratio");
    for e in &manifest.entries {
        let note = e.fallback_reason.as_deref().map(|r| format!("   [{r}]")).unwrap_or_default();
        println!(
            "{:>3}  {:<9} {:<8} {:>11} {:>11} {:>7}  {}{note}",
            e.id,
            e.tool,
            e.format,
            human_bytes(e.original_size),
            human_bytes(e.compressed_size),
            percent(e.compressed_size, e.original_size),
            e.path
        );
    }
    let original: u64 = manifest.entries.iter().map(|e| e.original_size).sum();
    let level = match manifest.level {
        1 => "store",
        2 => "medium",
        3 => "ultra",
        4 => "highultra",
        _ => "?",
    };
    println!(
        "\n{} files, {} -> {} ({}), level {level}, made by smartcomp {}",
        manifest.entries.len(),
        human_bytes(original),
        human_bytes(archive_bytes),
        percent(archive_bytes, original),
        manifest.far_version
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_times_are_readable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.5 KB");
        assert_eq!(human_bytes(93_202_081), "93.2 MB");
        assert_eq!(human_time(Duration::from_secs_f64(3.04)), "3.0s");
        assert_eq!(human_time(Duration::from_secs(200)), "3m 20s");
        assert_eq!(human_time(Duration::from_secs(3725)), "1h 02m 05s");
    }

    #[test]
    fn percent_handles_empty_input() {
        assert_eq!(percent(1, 0), "-");
        assert_eq!(percent(1, 4), "25.0%");
    }

    #[test]
    fn hidden_progress_tracks_files_without_panicking() {
        let p = Progress::new("t", 100, 2, false);
        p.started("dir/a.txt");
        p.started("b.txt");
        p.finished("dir/a.txt", 60);
        p.finished("b.txt", 40);
        p.line("done");
        p.finish();
        assert_eq!(p.state.lock().unwrap().done, 2);
    }
}
