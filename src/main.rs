#![allow(dead_code)] // some of this crate is a protocol library surface (validate, create_job, decode_document, …) not yet wired into the CLI

//! Local print and scan for an IPP/eSCL printer on your own network.
//!
//! No vendor app, no account, no subscription check, no internet connection.
//! See README.md for what this does and does not change about cartridge
//! authentication, which happens inside the printer's own firmware and is
//! out of any host program's reach.

mod discover;
mod escl;
mod ipp;
mod ippclient;
mod printjob;
mod pwgraster;
mod render;

use clap::{Parser, Subcommand};
use discover::Device;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "printer", about = "Local print and scan for an IPP/eSCL printer. No vendor app, no account, no internet.", version)]
struct Cli {
    /// Printer IP address, e.g. 192.168.1.50
    #[arg(long, global = true)]
    host: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Find printers on the network
    Find,
    /// Printer state, supplies and capabilities
    Status,
    /// Print one or more files (PDF or image)
    Print {
        files: Vec<PathBuf>,
        #[arg(short = 'n', long, default_value_t = 1)]
        copies: i32,
        #[arg(long, default_value = "A4")]
        paper: String,
        #[arg(long, default_value_t = 300)]
        dpi: u32,
        #[arg(long)]
        gray: bool,
        #[arg(long)]
        duplex: bool,
        #[arg(long)]
        name: Option<String>,
    },
    /// Scan from the flatbed or the document feeder
    Scan {
        #[arg(long)]
        feeder: bool,
        #[arg(long, default_value_t = 300)]
        dpi: u32,
        #[arg(long, default_value = "colour")]
        color: String,
        #[arg(long, default_value = "A4")]
        paper: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List jobs on the printer
    Jobs {
        #[arg(long)]
        completed: bool,
    },
    /// Cancel a job
    Cancel { job_id: i32 },
}

fn log(msg: &str) {
    println!("{msg}");
}

fn resolve_device(host: &Option<String>) -> anyhow::Result<Device> {
    if let Some(h) = host {
        let dev = discover::inspect_host(h, Duration::from_secs(3)).unwrap_or_else(|| {
            log("Nothing answered the usual ports, but keeping the address anyway in case the printer is waking up.");
            let mut d = Device::new(h);
            d.via = "given".into();
            d
        });
        discover::save_device(&dev).ok();
        return Ok(dev);
    }
    if let Some(dev) = discover::load_device() {
        return Ok(dev);
    }
    log("No printer saved yet, so looking for one...");
    let found = discover::discover(log);
    let Some(dev) = found.into_iter().next() else {
        anyhow::bail!(
            "No printer found.\n  - Check it's powered on and on the same network.\n  - Find its IP on the printer's network report, then run:\n      printer --host 192.168.1.50 status"
        );
    };
    discover::save_device(&dev).ok();
    log(&format!("Using and remembering {}", dev.label()));
    Ok(dev)
}

fn cmd_find() -> anyhow::Result<()> {
    let devices = discover::discover(log);
    if devices.is_empty() {
        println!("No printers found.");
        return Ok(());
    }
    for (i, d) in devices.iter().enumerate() {
        println!("\n{}. {}", i + 1, d.label());
        println!("   print: ipp://{}:{}{}", d.host, d.ipp_port, d.ipp_resource);
        if let Some(p) = d.scan_port {
            println!("   scan:  http://{}:{}{}", d.host, p, d.scan_resource);
        } else {
            println!("   scan:  not advertised");
        }
        println!("   settings page: http://{}/", d.host);
    }
    discover::save_device(&devices[0]).ok();
    println!("\nRemembered {}", devices[0].label());
    Ok(())
}

fn cmd_status(host: &Option<String>) -> anyhow::Result<()> {
    let dev = resolve_device(host)?;
    let printer = dev.printer();
    println!("Address: {}", printer.uri());
    let info = printer.info()?;
    println!("Model:   {}", if info.make_and_model.is_empty() { &info.name } else { &info.make_and_model });
    println!("State:   {}", if info.state.is_empty() { "unknown" } else { &info.state });
    println!("Accepting jobs: {}", if info.is_accepting { "yes" } else { "no" });
    println!("Colour:  {}", if info.color_supported { "yes" } else { "no" });

    if !info.ink.is_empty() {
        println!("\nSupplies (read from the printer; never used to block a print):");
        for s in &info.ink {
            println!("  {s}");
        }
    }
    println!("\nFormats: {}", if info.document_formats.is_empty() { "not stated".into() } else { info.document_formats.join(", ") });
    println!("Chosen:  {}", info.best_format().unwrap_or("none advertised"));

    if let Some(scanner) = dev.scanner() {
        println!("\nScanner: {}", scanner.base());
        match scanner.status() {
            Ok((state, adf)) => {
                println!("  State: {state}");
                if !adf.is_empty() {
                    println!("  Feeder: {adf}");
                }
            }
            Err(e) => println!("  unreachable - {e}"),
        }
        if let Ok(caps) = scanner.capabilities() {
            println!("  Sources: {}{}", if caps.platen { "flatbed " } else { "" }, if caps.feeder { "feeder" } else { "" });
            println!("  DPI: {:?}", caps.resolutions);
        }
    } else {
        println!("\nScanner: no eSCL endpoint known. Try: printer find");
    }
    Ok(())
}

fn cmd_print(host: &Option<String>, files: &[PathBuf], copies: i32, paper: &str, dpi: u32, gray: bool, duplex: bool, name: &Option<String>) -> anyhow::Result<()> {
    let dev = resolve_device(host)?;
    let printer = dev.printer();
    let options = printjob::PrintOptions { copies, paper: paper.to_string(), dpi, grayscale: gray, duplex, job_name: name.clone().unwrap_or_default() };

    let mut had_error = false;
    for path in files {
        match printjob::print_file(&printer, path, &options, log) {
            Ok(r) => {
                println!("Sent {}: job {}, {} page(s), {} bytes as {}", path.display(), r.job_id, r.pages, r.bytes_sent, r.document_format);
                for n in &r.notes {
                    println!("  note: {n}");
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                had_error = true;
            }
        }
    }
    if had_error {
        anyhow::bail!("one or more files failed to print");
    }
    Ok(())
}

fn cmd_scan(host: &Option<String>, feeder: bool, dpi: u32, color: &str, paper: &str, out: &Option<PathBuf>, name: &Option<String>) -> anyhow::Result<()> {
    let dev = resolve_device(host)?;
    let scanner = dev.scanner().ok_or_else(|| {
        anyhow::anyhow!("No scanner endpoint known for this printer. Run 'printer find' first so the eSCL port is discovered.")
    })?;

    let caps = scanner.capabilities()?;
    let mut source = if feeder { escl::SOURCE_FEEDER } else { escl::SOURCE_PLATEN }.to_string();
    if source == escl::SOURCE_FEEDER && !caps.feeder {
        println!("This scanner has no document feeder; using the flatbed instead.");
        source = escl::SOURCE_PLATEN.to_string();
    }

    let use_dpi = caps.nearest_resolution(dpi);
    if use_dpi != dpi {
        println!("{dpi} dpi is not supported; using {use_dpi} dpi.");
    }
    let mut color_mode = if color.starts_with("gr") { escl::COLOR_GRAY } else { escl::COLOR_RGB }.to_string();
    if !caps.color_modes.is_empty() && !caps.color_modes.contains(&color_mode) {
        color_mode = caps.color_modes.last().unwrap().clone();
        println!("Using colour mode {color_mode}.");
    }

    let (w, h) = if paper == "Letter" { escl::PAGE_LETTER } else { escl::PAGE_A4 };
    let settings = escl::ScanSettings { source: source.clone(), resolution: use_dpi, color_mode, document_format: "image/jpeg".into(), width: w.min(caps.max_width), height: h.min(caps.max_height) };

    println!("Scanning from the {} at {use_dpi} dpi...", source.to_lowercase());
    let pages = scanner.scan(&settings, |p| println!("  page {} received ({} bytes)", p.index + 1, p.data.len()))?;

    if pages.is_empty() {
        anyhow::bail!(if source == escl::SOURCE_FEEDER { "No pages came through. Is there paper in the feeder?" } else { "No page came through. Is a document on the glass with the lid down?" });
    }

    let out_dir = out.clone().unwrap_or_else(|| std::env::current_dir().unwrap().join("Scans"));
    std::fs::create_dir_all(&out_dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let base = name.clone().unwrap_or_else(|| format!("scan_{stamp}"));

    for page in &pages {
        let suffix = if pages.len() > 1 { format!("_{:02}", page.index + 1) } else { String::new() };
        let path = out_dir.join(format!("{base}{suffix}.{}", page.extension()));
        std::fs::write(&path, &page.data)?;
        println!("Saved {} ({} bytes)", path.display(), page.data.len());
    }
    Ok(())
}

fn cmd_jobs(host: &Option<String>, completed: bool) -> anyhow::Result<()> {
    let dev = resolve_device(host)?;
    let jobs = dev.printer().jobs(completed)?;
    if jobs.is_empty() {
        println!("No jobs.");
        return Ok(());
    }
    for j in jobs {
        let id = j.get("job-id").and_then(|v| v.as_i32()).unwrap_or(0);
        let state = j.get("job-state").and_then(|v| v.as_i32()).unwrap_or(0);
        let name = j.get("job-name").and_then(|v| v.as_str()).unwrap_or("");
        println!("{id:>6}  {:<18} {name}", ippclient::job_state_name(state));
    }
    Ok(())
}

fn cmd_cancel(host: &Option<String>, job_id: i32) -> anyhow::Result<()> {
    let dev = resolve_device(host)?;
    let ok = dev.printer().cancel(job_id)?;
    println!("Job {job_id}: {}", if ok { "cancelled" } else { "could not be cancelled" });
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Find) => cmd_find(),
        Some(Command::Status) | None => cmd_status(&cli.host),
        Some(Command::Print { files, copies, paper, dpi, gray, duplex, name }) => {
            if files.is_empty() {
                anyhow::bail!("give at least one file to print");
            }
            cmd_print(&cli.host, &files, copies, &paper, dpi, gray, duplex, &name)
        }
        Some(Command::Scan { feeder, dpi, color, paper, out, name }) => cmd_scan(&cli.host, feeder, dpi, &color, &paper, &out, &name),
        Some(Command::Jobs { completed }) => cmd_jobs(&cli.host, completed),
        Some(Command::Cancel { job_id }) => cmd_cancel(&cli.host, job_id),
    }
}
