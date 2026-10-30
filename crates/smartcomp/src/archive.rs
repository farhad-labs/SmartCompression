use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Component, Path, PathBuf};

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};

use anyhow::{bail, ensure, Context, Result};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::codecs::{self, Codec, Level};
use crate::container::{Reader, Writer};
use crate::select;
use crate::ui::Progress;
use crate::manifest::{payload_name, Entry, Manifest, MANIFEST_VERSION};

pub struct CreateOptions<'a> {
    pub level: Level,
    pub password: Option<&'a str>,
    pub verbose: bool,
    /// Developer override: use this codec id for every file.
    pub force_codec: Option<&'a str>,
    /// Show the live progress bar (it also hides itself when stderr is not a terminal).
    pub progress: bool,
}

/// Expands the inputs into (archive path, file path) pairs. A directory is stored
/// under its own name; paths always use '/'.
fn collect(inputs: &[PathBuf]) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    for input in inputs {
        let input = input.canonicalize().with_context(|| format!("cannot read {}", input.display()))?;
        let base = input.parent().map(Path::to_path_buf).unwrap_or_default();
        for e in WalkDir::new(&input).sort_by_file_name() {
            let e = e?;
            if !e.file_type().is_file() {
                continue;
            }
            let rel = e.path().strip_prefix(&base).unwrap_or(e.path());
            let name = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            files.push((name, e.path().to_path_buf()));
        }
    }
    if files.is_empty() {
        bail!("no files to archive");
    }
    Ok(files)
}

struct Compressed {
    codec: &'static dyn Codec,
    packed: Vec<u8>,
    fallback_reason: Option<String>,
    /// (spec, size) per candidate the probe tried, for the verbose log.
    probed: Vec<(&'static str, usize)>,
}

/// Picks a codec (probing the candidates), runs it, and falls back to UHARC `-mx`, then to
/// store, if it fails. Output that is not smaller than the input is replaced by the stored bytes.
fn compress_entry(data: &[u8], format: &str, opts: &CreateOptions) -> Result<Compressed> {
    let (first, mut ready, probed, mut reason) = match opts.force_codec {
        Some(spec) if opts.level != Level::Store => (codecs::by_spec(spec)?, None, Vec::new(), None),
        _ => {
            let choice = select::choose(select::classify(format, data), data, opts.level)?;
            (choice.codec, choice.packed, choice.probed, None)
        }
    };
    if first.id() != "store" {
        let mut chain = vec![first];
        if first.id() != "uharc-mx" {
            chain.push(codecs::by_id("uharc-mx")?);
        }
        for codec in chain {
            let result = match ready.take() {
                Some(p) => Ok(p),
                None => codecs::pack(codec, data, opts.level),
            };
            match result {
                Ok(packed) if packed.len() < data.len() => return Ok(Compressed { codec, packed, fallback_reason: reason, probed }),
                Ok(_) => {
                    reason = Some("expanded".to_string());
                    break;
                }
                Err(e) => {
                    if opts.verbose {
                        eprintln!("{} failed: {e:#}", codec.id());
                    }
                    reason = Some(format!("codec_error:{}", codec.id()));
                }
            }
        }
    }
    let store = codecs::by_id("store")?;
    Ok(Compressed { codec: store, packed: codecs::pack(store, data, Level::Store)?, fallback_reason: reason, probed })
}

struct Prepared {
    name: String,
    format: String,
    original_size: u64,
    blake3: String,
    c: Compressed,
}

fn prepare(name: &str, path: &Path, opts: &CreateOptions) -> Result<Prepared> {
    let data = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let format = crate::detect::detect(&data);
    let c = compress_entry(&data, &format, opts).with_context(|| format!("compressing {name}"))?;
    Ok(Prepared { name: name.to_string(), format, original_size: data.len() as u64, blake3: blake3::hash(&data).to_hex().to_string(), c })
}

pub fn create(archive: &Path, inputs: &[PathBuf], opts: &CreateOptions) -> Result<Manifest> {
    let files = collect(inputs)?;
    let sizes: Vec<u64> = files.iter().map(|(_, p)| fs::metadata(p).map_or(0, |m| m.len())).collect();
    let progress = Progress::new(&format!("Compressing [{}]", opts.level.name()), sizes.iter().sum(), files.len() as u64, opts.progress);
    let out = BufWriter::new(File::create(archive).with_context(|| format!("cannot create {}", archive.display()))?);
    let mut writer = Writer::new(out, opts.password)?;
    let mut entries = Vec::with_capacity(files.len());
    let failed = AtomicBool::new(false);

    // Files are compressed in parallel; this thread writes them to the archive in order.
    let (tx, rx) = mpsc::channel::<(usize, Result<Prepared>)>();
    let outcome: Result<()> = std::thread::scope(|scope| {
        let files = &files;
        let failed = &failed;
        let (progress, sizes) = (&progress, &sizes);
        scope.spawn(move || {
            // Longest-first: the slow coders make big files the critical path, so start them
            // early and let small files fill the gaps. Workers pull the next file from a shared counter.
            let mut order: Vec<usize> = (0..files.len()).collect();
            order.sort_by_key(|i| std::cmp::Reverse(sizes[*i]));
            let next = AtomicUsize::new(0);
            let (order, next) = (&order, &next);
            (0..rayon::current_num_threads()).into_par_iter().for_each_with(tx, |tx, _| loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= order.len() || failed.load(Ordering::Relaxed) {
                    break;
                }
                let i = order[k];
                let (name, path) = &files[i];
                progress.started(name);
                let r = prepare(name, path, opts);
                progress.finished(name, sizes[i]);
                if r.is_err() {
                    failed.store(true, Ordering::Relaxed);
                }
                let _ = tx.send((i, r));
            });
        });

        let mut pending = BTreeMap::new();
        let mut next = 0usize;
        for (i, r) in rx {
            pending.insert(i, r);
            while let Some(r) = pending.remove(&next) {
                let p = r?;
                let id = next as u32 + 1;
                let (offset, stored_size) = writer.add_blob(id, &p.c.packed)?;
                if opts.verbose {
                    let probe: Vec<String> = p.c.probed.iter().map(|(s, n)| format!("{s}={n}")).collect();
                    let probe = if probe.is_empty() { String::new() } else { format!(" [probe {}]", probe.join(" ")) };
                    progress.line(&format!("{} [{}]: {} -> {} ({}){probe}", p.name, p.format, p.original_size, p.c.packed.len(), p.c.codec.spec()));
                }
                let codec = p.c.codec;
                let params = format!("spec={} {}", codec.spec(), codec.params(opts.level)).trim_end().to_string();
                entries.push(Entry {
                    id,
                    path: p.name,
                    format: p.format,
                    tool: codec.id().into(),
                    tool_version: codec.version().into(),
                    tool_hash: codecs::tool_hash(codec, &params),
                    level: opts.level as u8,
                    original_size: p.original_size,
                    compressed_size: p.c.packed.len() as u64,
                    blake3: p.blake3,
                    method: match (codec.id(), &p.c.fallback_reason) {
                        (_, Some(_)) => "fallback",
                        ("store", None) => "store",
                        _ => "tool",
                    }
                    .into(),
                    stage: codec.id().into(),
                    payload: payload_name(id),
                    fallback_reason: p.c.fallback_reason,
                    offset,
                    stored_size,
                    params,
                });
                next += 1;
            }
        }
        Ok(())
    });
    progress.finish();
    outcome?;
    ensure!(entries.len() == files.len(), "archive creation was interrupted");

    let manifest = Manifest {
        version: MANIFEST_VERSION,
        far_version: env!("CARGO_PKG_VERSION").into(),
        level: opts.level as u8,
        entries,
    };
    writer.finish(&serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}

pub fn read_manifest(archive: &Path, password: Option<&str>) -> Result<(Reader, Manifest)> {
    let (reader, json) = Reader::open(archive, password)?;
    let manifest: Manifest = serde_json::from_slice(&json).context("manifest is not valid JSON")?;
    Ok((reader, manifest))
}

/// Joins an archive path onto `dest`, refusing anything that could escape it.
fn safe_join(dest: &Path, name: &str) -> Result<PathBuf> {
    let mut out = dest.to_path_buf();
    for part in name.split('/') {
        let p = Path::new(part);
        let ok = !part.is_empty() && p.components().count() == 1 && matches!(p.components().next(), Some(Component::Normal(_)));
        if !ok || part.contains('\\') || part.contains(':') {
            bail!("unsafe path in archive: {name:?}");
        }
        out.push(part);
    }
    Ok(out)
}

pub fn extract(archive: &Path, dest: &Path, password: Option<&str>, verbose: bool, show_progress: bool) -> Result<usize> {
    let (reader, manifest) = read_manifest(archive, password)?;
    // Validate every path before anything is written.
    let targets = manifest.entries.iter().map(|e| safe_join(dest, &e.path)).collect::<Result<Vec<_>>>()?;
    let reader = Mutex::new(reader);
    let progress = Progress::new("Extracting", manifest.entries.iter().map(|e| e.original_size).sum(), manifest.entries.len() as u64, show_progress);
    let progress = &progress;

    // Reading the archive is sequential, but decoding (slow for PPM-style coders) runs in parallel.
    let result = manifest.entries.par_iter().zip(targets.par_iter()).try_for_each(|(e, target)| -> Result<()> {
        progress.started(&e.path);
        let packed = reader.lock().unwrap().read_blob(e.id, e.offset, e.stored_size)?;
        let data = codecs::unpack(codecs::by_id(&e.tool)?, &packed, e.original_size as usize)?;
        drop(packed);
        if data.len() as u64 != e.original_size || blake3::hash(&data).to_hex().as_str() != e.blake3 {
            bail!("{}: checksum mismatch, archive is corrupted", e.path);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, &data).with_context(|| format!("cannot write {}", target.display()))?;
        if verbose {
            progress.line(&format!("{} ({} bytes)", e.path, data.len()));
        }
        progress.finished(&e.path, e.original_size);
        Ok(())
    });
    progress.finish();
    result?;
    Ok(manifest.entries.len())
}
