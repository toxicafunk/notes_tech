use anyhow::Context;
use clap::{Parser, Subcommand};
use ntbackup::export::{self, Resolution};
use ntbackup::header::Header;
use ntbackup::model::Folder;
use ntbackup::vault::VaultError;
use ntbackup::{logseq, OpenError};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use zeroize::Zeroizing;

/// Decrypt a Notes Tech backup (.ntbackup) and read its notes on a desktop.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what the header says. Needs no secret.
    Inspect { file: PathBuf },

    /// Write the decrypted JSON content. ⚠️ The output is plaintext.
    Decrypt {
        file: PathBuf,
        /// Where to write the JSON; `-` for standard output.
        #[arg(short, long)]
        output: PathBuf,
        #[command(flatten)]
        secret: SecretArgs,
    },

    /// Write one Markdown file per note. ⚠️ The output is plaintext.
    Export {
        file: PathBuf,
        /// The output directory — with --logseq, the root of a Logseq graph.
        #[arg(short, long)]
        output: PathBuf,
        /// Sync the notes into an existing Logseq graph, as pages.
        #[arg(long)]
        logseq: bool,
        /// Never ask for vault passphrases: notes of passphrase vaults are left out.
        #[arg(long)]
        skip_vaults: bool,
        /// Plain export: write into a non-empty directory.
        #[arg(long, conflicts_with = "logseq")]
        force: bool,
        /// --logseq: also rewrite or delete pages edited in Logseq since the last export.
        #[arg(long, requires = "logseq")]
        overwrite_edited: bool,
        /// --logseq: say what would change, and change nothing.
        #[arg(long, requires = "logseq")]
        dry_run: bool,
        #[command(flatten)]
        secret: SecretArgs,
    },
}

#[derive(clap::Args)]
struct SecretArgs {
    /// Read the backup secret from the first line of standard input instead of prompting.
    #[arg(long)]
    secret_stdin: bool,
}

const EXIT_OTHER: u8 = 1;
// 2 is clap's: a usage error.
const EXIT_REFUSED: u8 = 3;
const EXIT_WRONG_SECRET: u8 = 4;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(match e {
                OpenError::Refused(_) => EXIT_REFUSED,
                OpenError::WrongSecretOrDamaged => EXIT_WRONG_SECRET,
                OpenError::Other(_) => EXIT_OTHER,
            })
        }
    }
}

fn run(cli: Cli) -> Result<(), OpenError> {
    match cli.command {
        Command::Inspect { file } => {
            let bytes = ntbackup::read_file(&file)?;
            let header = Header::parse(&bytes).map_err(OpenError::Refused)?;
            println!("Notes Tech backup, format version {}", ntbackup::header::VERSION);
            println!(
                "Key derivation: Argon2id, t={}, m={} KiB, p={}",
                ntbackup::header::ITERATIONS,
                ntbackup::header::MEMORY_KIB,
                ntbackup::header::PARALLELISM
            );
            println!("Salt:  {}", hex(&header.salt));
            println!("Nonce: {}", hex(&header.nonce));
            println!(
                "Encrypted content: {} bytes",
                bytes.len() - ntbackup::header::HEADER_LEN
            );
            Ok(())
        }
        Command::Decrypt { file, output, secret } => {
            let bytes = ntbackup::read_file(&file)?;
            ntbackup::header::Header::parse(&bytes).map_err(OpenError::Refused)?;
            let typed = read_secret(&secret)?;
            let json = ntbackup::decrypt(&bytes, &typed)?;
            // Parsed only to refuse content of an unknown format; written as it was decrypted.
            ntbackup::model::Backup::parse(&json).map_err(OpenError::Other)?;
            if output == Path::new("-") {
                std::io::stdout()
                    .write_all(&json)
                    .map_err(|e| OpenError::Other(e.into()))?;
            } else {
                let parent = output
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let name = output
                    .file_name()
                    .context("output has no file name")
                    .map_err(OpenError::Other)?;
                export::write_private_file(
                    &parent.canonicalize().map_err(|e| OpenError::Other(e.into()))?,
                    Path::new(name),
                    &json,
                )
                .map_err(OpenError::Other)?;
                eprintln!("⚠️  {} holds your notes in clear.", output.display());
            }
            Ok(())
        }
        Command::Export {
            file,
            output,
            logseq,
            skip_vaults,
            force,
            overwrite_edited,
            dry_run,
            secret,
        } => {
            let bytes = ntbackup::read_file(&file)?;
            ntbackup::header::Header::parse(&bytes).map_err(OpenError::Refused)?;
            let typed = read_secret(&secret)?;
            let backup = ntbackup::open_backup(&bytes, &typed)?;
            drop(typed);
            let resolution =
                export::resolve(&backup, skip_vaults, &mut ask_vault_passphrase).map_err(OpenError::Other)?;
            if logseq {
                let report = logseq::sync(
                    &backup,
                    &resolution,
                    &output,
                    logseq::Options {
                        overwrite_edited,
                        dry_run,
                    },
                )
                .map_err(OpenError::Other)?;
                print_logseq_report(&report, &resolution, dry_run);
            } else {
                let summary = export::write_plain(
                    &backup,
                    &resolution,
                    &output,
                    force,
                    chrono::Utc::now().timestamp_millis(),
                )
                .map_err(OpenError::Other)?;
                println!("{} notes written to {}", summary.written, output.display());
                print_left_out(&resolution);
            }
            Ok(())
        }
    }
}

fn read_secret(args: &SecretArgs) -> Result<Zeroizing<String>, OpenError> {
    let secret = if args.secret_stdin {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| OpenError::Other(e.into()))?;
        Zeroizing::new(line.trim_end_matches(['\r', '\n']).to_owned())
    } else {
        Zeroizing::new(rpassword::prompt_password("Backup code or phrase: ").map_err(|e| OpenError::Other(e.into()))?)
    };
    if secret.is_empty() {
        return Err(OpenError::Other(anyhow::anyhow!("no secret given")));
    }
    Ok(secret)
}

fn ask_vault_passphrase(folder: &Folder, previous: Option<&VaultError>) -> Option<String> {
    if let Some(e) = previous {
        eprintln!("  {e}.");
    }
    let prompt = format!("Passphrase of the vault {:?} (empty to skip it): ", folder.name);
    match rpassword::prompt_password(prompt) {
        Ok(p) if !p.is_empty() => Some(p),
        _ => None,
    }
}

fn print_left_out(resolution: &Resolution) {
    let locked = resolution.locked_ids.len() - resolution.damaged;
    if locked > 0 {
        println!("{locked} notes of locked vaults left out");
    }
    if resolution.damaged > 0 {
        println!(
            "{} vault notes did not open (damaged data) and were left out",
            resolution.damaged
        );
    }
    if resolution.trashed > 0 {
        println!("{} notes in the trash left out", resolution.trashed);
    }
}

fn print_logseq_report(report: &logseq::Report, resolution: &Resolution, dry_run: bool) {
    use logseq::Action::*;
    if dry_run {
        for action in &report.actions {
            match action {
                Create { file } => println!("create   pages/{file}"),
                Update { file } => println!("update   pages/{file}"),
                Rename { from, to } => println!("rename   pages/{from} -> pages/{to}"),
                Delete { file } => println!("delete   pages/{file}"),
                SkipEdited { file, reason } => println!("skip     pages/{file} ({reason})"),
                Unchanged { .. } => {}
            }
        }
    }
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    let verb = if dry_run { "would be" } else { "" };
    println!(
        "Pages {verb} created: {}, updated: {}, renamed: {}, deleted: {}, unchanged: {}, skipped (edited): {}",
        report.count(|a| matches!(a, Create { .. })),
        report.count(|a| matches!(a, Update { .. })),
        report.count(|a| matches!(a, Rename { .. })),
        report.count(|a| matches!(a, Delete { .. })),
        report.count(|a| matches!(a, Unchanged { .. })),
        report.count(|a| matches!(a, SkipEdited { .. })),
    );
    print_left_out(resolution);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
