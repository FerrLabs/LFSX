mod client;
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
    }

    Ok(())
}
