//! Emit the DigitalReit deploy manifest to stdout.
//!
//! Run: `cargo run -p portrait-plan --example digital_reit_manifest`
//! The output is the covenant-set + lineage + deploy-order manifest a wallet
//! follows to stand up the two-covenant REIT waterfall in the correct order.

use portrait_plan::{Covenant, LineageEdge, LineagePlan};

fn main() {
    let plan = LineagePlan::new(
        "DigitalReit",
        vec![
            Covenant::new("DigitalReitToken", "token"),
            Covenant::new("DigitalReitSplitter", "splitter"),
        ],
        vec![LineageEdge::new(
            "DigitalReitToken",    // parent: the REIT share registry / declarer
            "DigitalReitSplitter", // child: the payment waterfall
            "parent_kov_id",       // child state field that binds the parent's covenant ID
        )],
    );

    let manifest = plan.manifest().expect("DigitalReit plan is valid");
    print!("{}", manifest.to_json());
}
