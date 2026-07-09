//! Print the x-only (BIP-340) public key for a wallet key file.
//!
//! Usage: KCP_KEY_FILE=path cargo run -p kcp-common --example pubkey --features wrpc
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::path::Path;

use kcp_common::wallet::{Prefix, Wallet};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key_file = std::env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required")?;
    let w = Wallet::load(Path::new(&key_file), 0, Prefix::Testnet)?;
    let xonly: [u8; 32] = w.keypair.x_only_public_key().0.serialize();
    println!("address: {}", w.address_string());
    println!("xonly_pubkey: {}", hex::encode(xonly));
    Ok(())
}
