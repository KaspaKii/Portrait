//! `kcp` — Kii Wizard CLI for the kaspa-compliance-patterns library.
//!
//! **Pre-production, unaudited.** Generated projects are testnet-only scaffolds.
//!
//! Usage:
//!   kcp scaffold vault --threshold 2 --keys KEY1,KEY2 --workspace-path /path/to/repo
//!   kcp scaffold timelock --deadline 1000000 --workspace-path /path/to/repo
//!   kcp scaffold composite --deadline 1000000 --threshold 2 --n 3 --workspace-path /path/to/repo
//!   kcp scaffold ktt-token --token-name MyToken --initial-supply 1000000 --workspace-path /path/to/repo
//!   kcp scaffold sealed-lineage --subject "MyLineage" --workspace-path /path/to/repo
//!   kcp scaffold transferable-record --record-type "LandTitle" --workspace-path /path/to/repo
//!   kcp scaffold paired-attestation --subject-label "ServiceAgreement" --workspace-path /path/to/repo
//!   kcp scaffold governance --title "Fund auditor" --threshold 2 --n 3 --workspace-path /path/to/repo
//!   kcp scaffold vesting --start 100000 --duration 86400 --total-amount 1000000 --workspace-path /path/to/repo
//!   kcp scaffold yield-vault --initial-deposit 1000000 --yield-amount 100000 --workspace-path /path/to/repo
//!   kcp scaffold pq-anchor --workspace-path /path/to/repo
//!   kcp new --from-solidity erc20 --name MyToken --symbol MYT --supply 1000000 --workspace-path /path/to/repo
//!
//! Until the library is published on crates.io (v0.2+), `--workspace-path` is
//! required so the generated Cargo.toml can reference the local library via
//! path dependencies.

use clap::{Parser, Subcommand};
use kcp::scaffold;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "kcp",
    about = "Kii Wizard CLI — scaffold Kaspa covenant projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a new Kaspa covenant project.
    Scaffold(ScaffoldArgs),
    /// Migrate from an ERC20/Solidity pattern to a Kaspa covenant (pre-production, unaudited).
    New(NewArgs),
}

#[derive(clap::Args)]
struct ScaffoldArgs {
    #[command(subcommand)]
    pattern: PatternArgs,
}

#[derive(Subcommand)]
enum PatternArgs {
    /// Scaffold a P2SH multisig vault covenant.
    Vault(VaultArgs),
    /// Scaffold a single-key P2SH DAA-height timelock covenant.
    Timelock(TimelockArgs),
    /// Scaffold a composite All([TimelockHeight, MultiSig]) covenant.
    Composite(CompositeArgs),
    /// Scaffold a KCC20-shape regulated-token state-machine demo (mint → transfer → burn).
    KttToken(KttTokenArgs),
    /// Scaffold a sealed-lineage append-only evidence chain demo (genesis → append → validate).
    SealedLineage(SealedLineageArgs),
    /// Scaffold a transferable-record ownership-transfer chain demo (genesis → transfer → validate).
    TransferableRecord(TransferableRecordArgs),
    /// Scaffold a paired-attestation two-party mate-proof demo (commit → build-proof → verify).
    PairedAttestation(PairedAttestationArgs),
    /// Scaffold a governance proposal → vote → timelock → execute cycle.
    Governance(GovernanceArgs),
    /// Scaffold a linear DAA-height vesting schedule demo.
    Vesting(VestingArgs),
    /// Scaffold a yield vault shares/assets accounting demo (ERC4626-equivalent).
    YieldVault(YieldVaultArgs),
    /// Scaffold a KIP-16 tag-0x21 post-quantum credential anchor script assembly demo.
    PqAnchor(PqAnchorArgs),
}

#[derive(clap::Args)]
struct VaultArgs {
    /// Number of signatures required (threshold ≤ number of keys).
    #[arg(long, default_value = "2")]
    threshold: u8,

    /// Comma-separated list of public key labels (e.g. KEY1,KEY2).
    /// In v0 these are used as comments in the generated code; replace
    /// `test_keypair(0xNN)` calls with your real keys before use.
    #[arg(long, default_value = "KEY1,KEY2")]
    keys: String,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-vault")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct TimelockArgs {
    /// DAA height at or after which the controller key may spend.
    /// Must be > 0 and < 500_000_000_000 (height-based CLTV).
    #[arg(long, default_value = "1000000")]
    deadline: u64,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-timelock")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct CompositeArgs {
    /// DAA height at or after which spending is allowed (height-based CLTV).
    #[arg(long, default_value = "1000000")]
    deadline: u64,

    /// Number of multisig signatures required (threshold ≤ n).
    #[arg(long, default_value = "2")]
    threshold: u8,

    /// Total number of multisig keys.
    #[arg(long, default_value = "3")]
    n: usize,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-composite")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct KttTokenArgs {
    /// Token name used in generated comments (display only in v0).
    #[arg(long, default_value = "MyToken")]
    token_name: String,

    /// Initial supply minted in the demo.
    #[arg(long, default_value = "1000000")]
    initial_supply: u64,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-ktt-token")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct SealedLineageArgs {
    /// Lineage subject label used in generated comments.
    #[arg(long, default_value = "MyLineage")]
    subject: String,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-sealed-lineage")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct TransferableRecordArgs {
    /// Record type label used in generated comments.
    #[arg(long, default_value = "MyRecord")]
    record_type: String,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-transferable-record")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct PairedAttestationArgs {
    /// Attestation subject label used in generated comments.
    #[arg(long, default_value = "MyAttestation")]
    subject_label: String,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-paired-attestation")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct GovernanceArgs {
    #[arg(long, default_value = "Fund auditor")]
    title: String,
    #[arg(long, default_value = "2")]
    threshold: u8,
    #[arg(long, default_value = "3")]
    n: usize,
    #[arg(long, default_value = "1000")]
    voting_window: u64,
    #[arg(long, default_value = "500")]
    timelock_delay: u64,
    #[arg(long, default_value = "./kii-covenants-out/my-governance")]
    out: PathBuf,
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct VestingArgs {
    #[arg(long, default_value = "100000")]
    start: u64,
    #[arg(long, default_value = "86400")]
    duration: u64,
    #[arg(long, default_value = "1000000")]
    total_amount: u64,
    #[arg(long, default_value = "./kii-covenants-out/my-vesting")]
    out: PathBuf,
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct YieldVaultArgs {
    #[arg(long, default_value = "1000000")]
    initial_deposit: u64,
    #[arg(long, default_value = "100000")]
    yield_amount: u64,
    #[arg(long, default_value = "./kii-covenants-out/my-yield-vault")]
    out: PathBuf,
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct PqAnchorArgs {
    #[arg(long, default_value = "./kii-covenants-out/my-pq-anchor")]
    out: PathBuf,
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct NewArgs {
    /// Which Solidity pattern to migrate from.
    #[command(subcommand)]
    pattern: FromSolidityPattern,
}

#[derive(Subcommand)]
enum FromSolidityPattern {
    /// Migrate from ERC20 → KTT (Kaspa Trust Token).
    Erc20(Erc20MigrationArgs),
    /// Migrate from Ownable → single-key UTXO ownership record.
    Ownable(OwnableMigrationArgs),
    /// Migrate from TimelockController → kcp-governance TimelockAction.
    Timelock(TimelockMigrationArgs),
    /// Migrate from ERC4626/Escrow → kcp-vault SpendCondition.
    Vault(VaultMigrationArgs),
}

#[derive(clap::Args)]
struct Erc20MigrationArgs {
    /// Token name (e.g. "Acme Token").
    #[arg(long, default_value = "MyToken")]
    name: String,

    /// Ticker symbol (e.g. "MYT").
    #[arg(long, default_value = "MYT")]
    symbol: String,

    /// Decimal places for display (does not affect on-chain amount).
    #[arg(long, default_value = "8")]
    decimals: u8,

    /// Initial token supply in the smallest representable unit.
    #[arg(long, default_value = "1000000")]
    supply: u64,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-erc20-migration")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    /// Required until v0.2 publishes to crates.io.
    #[arg(long)]
    workspace_path: PathBuf,

    /// Run `cargo test` on the generated project to validate KTT state-machine
    /// transitions offline before deploying to TN10.
    #[arg(long, default_value_t = false)]
    run_tests: bool,
}

#[derive(clap::Args)]
struct OwnableMigrationArgs {
    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-ownable-migration")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct TimelockMigrationArgs {
    /// Minimum delay in DAA heights (≈ 1 s per height at 1 BPS).
    #[arg(long, default_value = "86400")]
    min_delay_daa: u64,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-timelock-migration")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    #[arg(long)]
    workspace_path: PathBuf,
}

#[derive(clap::Args)]
struct VaultMigrationArgs {
    /// Timelock deadline in DAA heights for the generated vault condition.
    #[arg(long, default_value = "1000000")]
    deadline_daa: u64,

    /// Output directory for the generated project.
    #[arg(long, default_value = "./kii-covenants-out/my-vault-migration")]
    out: PathBuf,

    /// Absolute path to the kaspa-compliance-patterns workspace root.
    #[arg(long)]
    workspace_path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scaffold(s) => match s.pattern {
            PatternArgs::Vault(args) => {
                let keys: Vec<String> =
                    args.keys.split(',').map(|k| k.trim().to_string()).collect();
                let cfg = scaffold::vault::VaultConfig {
                    threshold: args.threshold,
                    keys,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::vault::generate(&cfg)?;
                println!(
                    "Generated vault scaffold at {:?}\n\
                     Replace test_keypair(0xNN) with your real keys before use.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::Timelock(args) => {
                let cfg = scaffold::timelock::TimelockConfig {
                    deadline: args.deadline,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::timelock::generate(&cfg)?;
                println!(
                    "Generated timelock scaffold at {:?}\n\
                     Replace test_keypair(0xA1) with your real controller keypair.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::Composite(args) => {
                let cfg = scaffold::composite::CompositeConfig {
                    deadline: args.deadline,
                    threshold: args.threshold,
                    n: args.n,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::composite::generate(&cfg)?;
                println!(
                    "Generated composite scaffold at {:?}\n\
                     Replace test_keypair(0xNN) with your real keypairs before use.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::KttToken(args) => {
                let cfg = scaffold::ktt_token::KttTokenConfig {
                    token_name: args.token_name,
                    initial_supply: args.initial_supply,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::ktt_token::generate(&cfg)?;
                println!(
                    "Generated ktt-token scaffold at {:?}\n\
                     Replace synthetic owner identifiers with real Schnorr public keys.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::SealedLineage(args) => {
                let cfg = scaffold::sealed_lineage::SealedLineageConfig {
                    subject: args.subject,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::sealed_lineage::generate(&cfg)?;
                println!(
                    "Generated sealed-lineage scaffold at {:?}\n\
                     Replace synthetic blinds with secure random bytes before use.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::TransferableRecord(args) => {
                let cfg = scaffold::transferable_record::TransferableRecordConfig {
                    record_type: args.record_type,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::transferable_record::generate(&cfg)?;
                println!(
                    "Generated transferable-record scaffold at {:?}\n\
                     Replace synthetic controller keys with real Schnorr public keys.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::PairedAttestation(args) => {
                let cfg = scaffold::paired_attestation::PairedAttestationConfig {
                    subject_label: args.subject_label,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::paired_attestation::generate(&cfg)?;
                println!(
                    "Generated paired-attestation scaffold at {:?}\n\
                     Replace synthetic blind shares with CSPRNG-derived bytes before use.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::Governance(args) => {
                let cfg = scaffold::governance::GovernanceConfig {
                    title: args.title,
                    threshold: args.threshold,
                    n: args.n,
                    voting_window: args.voting_window,
                    timelock_delay: args.timelock_delay,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::governance::generate(&cfg)?;
                println!(
                    "Generated governance scaffold at {:?}\n\
                     Replace synthetic keys with real Schnorr x-only public keys.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::Vesting(args) => {
                let cfg = scaffold::vesting::VestingConfig {
                    start: args.start,
                    duration: args.duration,
                    total_amount: args.total_amount,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::vesting::generate(&cfg)?;
                println!(
                    "Generated vesting scaffold at {:?}\n\
                     Replace synthetic beneficiary key with a real Schnorr x-only public key.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::YieldVault(args) => {
                let cfg = scaffold::yield_vault::YieldVaultConfig {
                    initial_deposit: args.initial_deposit,
                    yield_amount: args.yield_amount,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::yield_vault::generate(&cfg)?;
                println!(
                    "Generated yield-vault scaffold at {:?}\n\
                     Pair with kcp-ktt-token for on-chain share tokens.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            PatternArgs::PqAnchor(args) => {
                let cfg = scaffold::pq_anchor::PqAnchorConfig {
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::pq_anchor::generate(&cfg)?;
                println!(
                    "Generated pq-anchor scaffold at {:?}\n\
                     Replace synthetic proof fields with real RISC Zero guest output.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
        },
        Commands::New(new_args) => match new_args.pattern {
            FromSolidityPattern::Erc20(args) => {
                let cfg = scaffold::from_solidity_erc20::FromSolidityErc20Config {
                    name: args.name,
                    symbol: args.symbol,
                    decimals: args.decimals,
                    initial_supply: args.supply,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::from_solidity_erc20::generate(&cfg)?;
                if args.run_tests {
                    let status = std::process::Command::new("cargo")
                        .arg("test")
                        .arg("--manifest-path")
                        .arg(cfg.out_dir.join("Cargo.toml"))
                        .status()
                        .map_err(|e| anyhow::anyhow!("failed to invoke cargo: {e}"))?;
                    if !status.success() {
                        anyhow::bail!(
                            "cargo test failed on generated project — fix the test failures before deploying."
                        );
                    }
                    println!("cargo test: PASS — KTT state-machine transitions validated offline.");
                }
                println!(
                    "Generated ERC20→KTT migration scaffold at {:?}\n\
                     Replace synthetic keys with real Schnorr x-only public keys.\n\
                     To deploy on TN10, encode KttState values into a transaction via kcp-ktt-token wrpc.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            FromSolidityPattern::Ownable(args) => {
                let cfg = scaffold::from_solidity_ownable::FromSolidityOwnableConfig {
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::from_solidity_ownable::generate(&cfg)?;
                println!(
                    "Generated Ownable→OwnershipRecord scaffold at {:?}\n\
                     Replace synthetic keys with real 32-byte x-only Schnorr public keys.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir
                );
                Ok(())
            }
            FromSolidityPattern::Timelock(args) => {
                let cfg = scaffold::from_solidity_timelock::FromSolidityTimelockConfig {
                    min_delay_daa: args.min_delay_daa,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::from_solidity_timelock::generate(&cfg)?;
                println!(
                    "Generated TimelockController→kcp-governance scaffold at {:?}\n\
                     Min delay: {} DAA heights (≈ {} seconds at 1 BPS).\n\
                     Replace synthetic keys with real Schnorr x-only keys.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir, cfg.min_delay_daa, cfg.min_delay_daa
                );
                Ok(())
            }
            FromSolidityPattern::Vault(args) => {
                let cfg = scaffold::from_solidity_vault::FromSolidityVaultConfig {
                    deadline_daa: args.deadline_daa,
                    out_dir: args.out,
                    workspace_path: args.workspace_path,
                };
                scaffold::from_solidity_vault::generate(&cfg)?;
                println!(
                    "Generated ERC4626/Escrow→kcp-vault scaffold at {:?}\n\
                     Timelock deadline: {} DAA heights.\n\
                     Replace synthetic keys and deadline with real values.\n\
                     Pre-production, unaudited, testnet-only.",
                    cfg.out_dir, cfg.deadline_daa
                );
                Ok(())
            }
        },
    }
}
