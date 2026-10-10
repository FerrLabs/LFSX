mod client;
mod dedupe;
mod doctor;
mod gc;
mod summary;

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
            println!(
                "{}",
                summary::gc(&gc::run(&server, &repo, dry_run)?, dry_run)
            );
        }
        Command::Dedupe { repo, dry_run } => {
            let report = dedupe::run(&server, &repo, dry_run)?;
            println!("{}", summary::dedupe(&report, dry_run));
        }
        Command::Compress { repo, dry_run } => {
            let report = dedupe::compress(&server, &repo, dry_run)?;
            println!("{}", summary::compress(&report, dry_run));
        }
        Command::Verify { repo } => {
            let audit = summary::verify(&dedupe::verify(&server, &repo)?);
            print!("{}", audit.text);

            if !audit.clean {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
