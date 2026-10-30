use clap::Parser;

fn main() {
    // Hidden worker mode: precomp runs in a child process (see codecs/precomp.rs).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("__precomp") {
        std::process::exit(smartcomp::codecs::precomp_child_main(&args[2..]));
    }
    if let Err(e) = smartcomp::cli::run(smartcomp::cli::Cli::parse()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
