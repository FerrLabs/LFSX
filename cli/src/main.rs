mod client;
mod dedupe;
mod doctor;
mod gc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use client::{Server, resolve_token};

#[derive(Parser)]
#[command(
    name = "lfsx",
    version,
    about = "Companion for a self-hosted LFSX server"
)]
struct Cli {
    #[arg(long, env = "LFSX_URL")]
    url: String,

    #[arg(long, env = "LFSX_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Doctor {
        #[arg(long)]
        repo: Option<String>,
    },
    Gc {
        #[arg(long)]
        repo: String,

        #[arg(long)]
        dry_run: bool,
    },
    Dedupe {
        #[arg(long)]
        repo: String,

        #[arg(long)]
        dry_run: bool,
    },
    Compress {
        #[arg(long)]
        repo: String,

        #[arg(long)]
        dry_run: bool,
    },
    Verify {
        #[arg(long)]
        repo: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let server = Server::new(&cli.url, resolve_token(cli.token))?;

    match cli.command {
        Command::Doctor { repo } => {
            let report = doctor::run(&server, repo.as_deref())?;
            report.print();

            if !report.healthy() {
                std::process::exit(1);
            }
        }
        Command::Gc { repo, dry_run } => {
            let report = gc::run(&server, &repo, dry_run)?;
            let swept = report["swept"].as_u64().unwrap_or_default();
            let bytes = report["bytes"].as_u64().unwrap_or_default();
            let kept = report["within_grace"].as_u64().unwrap_or_default();

            println!(
                "{} {swept} objects, {:.1} GiB{}",
                if dry_run { "would free" } else { "freed" },
                bytes as f64 / 1_073_741_824.0,
                if kept > 0 {
                    format!(", {kept} left alone inside the grace period")
                } else {
                    String::new()
                }
            );
        }
        Command::Dedupe { repo, dry_run } => {
            let report = dedupe::run(&server, &repo, dry_run)?;
            let adopted = report["adopted"].as_u64().unwrap_or_default();
            let linked = report["linked"].as_u64().unwrap_or_default();
            let reclaimed = report["reclaimed"].as_u64().unwrap_or_default();
            let refused = report["refused"].as_u64().unwrap_or_default();

            println!(
                "{} {adopted} objects into the shared store, linked {linked}, {} {:.2} GiB{}",
                if dry_run { "would move" } else { "moved" },
                if dry_run { "freeing" } else { "freed" },
                reclaimed as f64 / 1_073_741_824.0,
                if refused > 0 {
                    format!(" — {refused} refused, see the server log")
                } else {
                    String::new()
                }
            );
        }
        Command::Compress { repo, dry_run } => {
            let report = dedupe::compress(&server, &repo, dry_run)?;
            let compressed = report["compressed"].as_u64().unwrap_or_default();
            let already = report["already"].as_u64().unwrap_or_default();
            let left = report["left_alone"].as_u64().unwrap_or_default();
            let before = report["before"].as_u64().unwrap_or_default();
            let after = report["after"].as_u64().unwrap_or_default();

            println!(
                "{} {compressed} objects: {:.2} GiB -> {:.2} GiB ({}% smaller), {already} already \
             compressed, {left} left as they were",
                if dry_run {
                    "would compress"
                } else {
                    "compressed"
                },
                before as f64 / 1_073_741_824.0,
                after as f64 / 1_073_741_824.0,
                100 - after.saturating_mul(100).checked_div(before).unwrap_or(100),
            );
        }
        Command::Verify { repo } => {
            let report = dedupe::verify(&server, &repo)?;
            let checked = report["checked"].as_u64().unwrap_or_default();
            let bytes = report["bytes"].as_u64().unwrap_or_default();
            let corrupt = report["corrupt"].as_array().cloned().unwrap_or_default();
            let unreadable = report["unreadable"].as_array().cloned().unwrap_or_default();

            println!(
                "read {checked} objects, {:.2} GiB",
                bytes as f64 / 1_073_741_824.0
            );

            for (label, oids) in [("corrupt", &corrupt), ("unreadable", &unreadable)] {
                if oids.is_empty() {
                    continue;
                }

                println!("{label}:");
                for oid in oids.iter() {
                    println!("  {}", oid.as_str().unwrap_or_default());
                }
            }

            let incomplete = report["incomplete"].as_bool().unwrap_or_default();
            if incomplete {
                println!(
                    "part of the repository could not be listed — this audit is not a clean bill"
                );
            }

            if incomplete || !corrupt.is_empty() || !unreadable.is_empty() {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
