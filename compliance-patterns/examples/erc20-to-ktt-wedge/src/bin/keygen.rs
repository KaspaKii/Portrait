//! Generate a fresh secp256k1 keypair and print the TN10 address.
//!
//! Usage:
//!   cargo run --manifest-path examples/erc20-to-ktt-wedge/Cargo.toml \
//!             --bin keygen -- /path/to/output.key
//!
//! Writes a 64-char hex private key to the given file and prints the
//! corresponding testnet:10 address. Fund that address from the TN10 faucet
//! before running the erc20-to-ktt-wedge demo.

use std::path::PathBuf;

use kaspa_bip32::{
    secp256k1::{rand, Keypair, Secp256k1},
    SecretKey,
};
use kaspa_addresses::{Address, Prefix, Version};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let scratchpad = std::env::temp_dir().join("kii-wedge-wallet.key");
            eprintln!("no output path given; writing to {}", scratchpad.display());
            scratchpad
        });

    // Generate random keypair.
    let secp = Secp256k1::new();
    let (secret_key, _) = secp.generate_keypair(&mut rand::thread_rng());
    let hex_key = hex::encode(secret_key.secret_bytes());

    // Derive TN10 address: x-only public key → kaspa address.
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let xonly = keypair.x_only_public_key().0;
    let address = Address::new(Prefix::Testnet, Version::PubKey, &xonly.serialize());

    // Write key file (one line, no newline issues).
    std::fs::write(&out_path, &hex_key)?;
    println!("key file:  {}", out_path.display());
    println!("address:   {address}");
    println!();
    println!("Fund this address on TN10, then run:");
    println!(
        "  KCP_NODE_URL=ws://localhost:17210 \\\n  \
         KCP_KEY_FILE={} \\\n  \
         cargo run --manifest-path examples/erc20-to-ktt-wedge/Cargo.toml",
        out_path.display()
    );

    Ok(())
}
