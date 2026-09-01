use kcp_vesting::error::VestingError;
use kcp_vesting::schedule::VestingSchedule;

fn beneficiary(seed: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = seed;
    k
}

#[test]
fn duration_zero_rejected() {
    assert_eq!(
        VestingSchedule::new(beneficiary(1), 1000, 0, 1_000_000).unwrap_err(),
        VestingError::DurationZero
    );
}

#[test]
fn nothing_releasable_before_start() {
    let s = VestingSchedule::new(beneficiary(1), 1000, 500, 1_000_000).unwrap();
    assert_eq!(s.releasable(999), 0);
    assert_eq!(s.releasable(1000), 0); // at start: 0 elapsed
}

#[test]
fn fully_vested_after_end() {
    let s = VestingSchedule::new(beneficiary(1), 1000, 500, 1_000_000).unwrap();
    assert_eq!(s.releasable(1500), 1_000_000);
    assert_eq!(s.releasable(9999), 1_000_000);
}

#[test]
fn linear_vesting_at_halfway() {
    let s = VestingSchedule::new(beneficiary(1), 1000, 1000, 1_000_000).unwrap();
    // At height 1500 (500 elapsed out of 1000): expect 500_000
    assert_eq!(s.vested_amount(1500), 500_000);
    assert_eq!(s.releasable(1500), 500_000);
}

#[test]
fn release_updates_schedule() {
    let s = VestingSchedule::new(beneficiary(1), 0, 1000, 1_000_000).unwrap();
    let (s2, amount) = s.release(500).unwrap();
    assert_eq!(amount, 500_000);
    assert_eq!(s2.released, 500_000);
    // Second release at 750: total vested = 750_000, already released 500_000
    let (s3, amount2) = s2.release(750).unwrap();
    assert_eq!(amount2, 250_000);
    assert_eq!(s3.released, 750_000);
}

#[test]
fn nothing_to_release_before_start_errors() {
    let s = VestingSchedule::new(beneficiary(1), 1000, 500, 1_000_000).unwrap();
    assert_eq!(s.release(999).unwrap_err(), VestingError::NothingToRelease);
}

#[test]
fn release_after_full_vest_then_nothing() {
    let s = VestingSchedule::new(beneficiary(1), 0, 100, 1_000).unwrap();
    let (s2, amount) = s.release(200).unwrap();
    assert_eq!(amount, 1_000);
    // Nothing left to release
    assert_eq!(s2.release(300).unwrap_err(), VestingError::NothingToRelease);
}

#[test]
fn end_height_correct() {
    let s = VestingSchedule::new(beneficiary(1), 1000, 500, 0).unwrap();
    assert_eq!(s.end(), 1500);
}

#[test]
fn serde_round_trip() {
    let s = VestingSchedule::new(beneficiary(42), 5000, 10000, 100_000_000).unwrap();
    let json = serde_json::to_string(&s).unwrap();
    let back: VestingSchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}
