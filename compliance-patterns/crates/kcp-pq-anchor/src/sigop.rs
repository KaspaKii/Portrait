/// Returns the sigOpCount required for a KIP-16 tag-0x21 spend.
///
/// Tag-0x21 verification uses approximately 25 million script units.
/// sigOpCount is a u8 (max 255). Budget = sigOpCount × 100_000 + 9_999;
/// at 255 that is ~25.5 million units, sufficient for RISC Zero verification.
pub const fn sigop_count_for_pq_verify() -> u8 {
    255
}
