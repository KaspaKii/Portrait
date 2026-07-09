/* kaspa-compliance-patterns — covenant wizard generator.
   Pure, deterministic, no DOM access. Usable from the browser (wizard.html)
   AND from node (module.exports guard) so the exact shipping code can be
   driven by `node` and verified against the real `portrait check`.

   Every pattern is derived from a KNOWN-GOOD source that `portrait check`
   accepts: the `portrait new` templates (counter, escrow, treasury), the
   library covenants TimeVault, StreamingVesting, Subscription,
   SpendingLimitVault, the token sources SimpleToken / PausableToken /
   KycGatedTransfer, and the cross-layer vProg sources ComplianceCredential /
   BatchRollup / ProofOfReserves (CSCI shape). Feature toggles map to REAL
   named invariants / `requires` guards used by those sources — nothing
   invented. The vprog pattern additionally passes the real
   `portrait atelier-build` (RISC Zero guest skeleton emitted, exit 0).

   HONESTY: output is a STARTING POINT only. Pre-production, unaudited,
   testnet-only. The generated header says so, and the UI repeats it. */
(function () {
  "use strict";

  /* ── Pattern + feature metadata (consumed by wizard.html and by node) ── */

  var PATTERNS = [
    {
      id: "counter",
      label: "Counter",
      defaultName: "MyCounter",
      desc: "A minimal single-transition covenant — the smallest real covenant shape and a good first compile. By default it accepts any signed delta; add the strict +1 toggle to make it forward-only.",
      source: "portrait new --template counter",
      features: [
        {
          id: "strict_sequence",
          label: "Strict +1 sequence",
          invariant: "monotonic_seq",
          desc: "Every state-mutating spend must advance `seq` by exactly one. The checker rejects any edit that breaks the +1 step."
        },
        {
          id: "owner_auth",
          label: "Owner authorisation",
          invariant: "authorized",
          desc: "Only the committed owner key may bump the counter (checkSig against committed state, never a caller-supplied key)."
        }
      ]
    },
    {
      id: "vault",
      label: "Time-locked vault",
      defaultName: "MyVault",
      desc: "Custody covenant: the owner may release funds only once a committed time bucket has passed. One-shot flag prevents double-spend across paths.",
      source: "library/custody/time-vault/TimeVault.portrait",
      features: [
        {
          id: "recovery_clawback",
          label: "Recovery clawback path",
          invariant: "checkSig(auth, recovery) guard",
          desc: "Adds a second committed cold key that can claw the funds back at any time before release — the break-glass path."
        },
        {
          id: "temporal_guard",
          label: "Enforced time gate",
          invariant: "temporal_guard",
          desc: "Declares the time gate (`now_bucket >= unlock_bucket`) as a checked invariant: an edit that drops the gate fails `portrait check`. Structural shape match, not a wall-clock proof."
        },
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that every state-mutating spend must checkSig against a committed key. An added transition without one is rejected."
        }
      ]
    },
    {
      id: "escrow",
      label: "Two-party escrow",
      defaultName: "MyEscrow",
      desc: "Deadline-gated conditional payment: seller releases (happy path) XOR buyer refunds after the deadline. One-shot settled flag.",
      source: "portrait new --template escrow",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that release/refund authority is checked against the COMMITTED buyer/seller keys — a transition without a committed-key checkSig is rejected."
        },
        {
          id: "temporal_guard",
          label: "Enforced refund deadline",
          invariant: "temporal_guard",
          desc: "Declares the refund gate (`now_bucket >= deadline`) as a checked invariant. Structural shape match on the guard, not a wall-clock proof."
        }
      ]
    },
    {
      id: "treasury",
      label: "Multisig treasury",
      defaultName: "MyTreasury",
      desc: "A 2-of-2 treasury: the balance moves only when BOTH committed signers authorise in the same transaction.",
      source: "portrait new --template treasury",
      features: [
        {
          id: "multisig_threshold",
          label: "Enforced 2-key threshold",
          invariant: "multisig_threshold",
          desc: "Declares that every state-mutating spend must carry at least 2 distinct committed-key signatures. An edit down to one signer fails the checker."
        },
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares committed-key authorisation as a checked invariant (spend already checkSigs both committed signer keys)."
        },
        {
          id: "non_negative",
          label: "Non-negative amounts",
          invariant: "non_negative_amount",
          desc: "Declares `amount >= 0` as a checked invariant: a future edit that drops the bound fails `portrait check`."
        },
        {
          id: "spending_cap",
          label: "Per-transaction spending cap",
          invariant: "spending_cap",
          desc: "Adds a committed `limit` field and requires `amount <= limit` on every spend, enforced by the checker (per-tx cap, NOT a time-windowed rate limit)."
        }
      ]
    },
    {
      id: "vesting",
      label: "Vesting stream",
      defaultName: "MyVesting",
      desc: "Single-recipient grant drawn down over time: cumulative withdrawals accumulate in `supply` and can never exceed the committed `total` ceiling.",
      source: "library/finance/streaming/StreamingVesting.portrait",
      features: [
        {
          id: "bounded_supply",
          label: "Enforced grant ceiling",
          invariant: "bounded_supply",
          desc: "Declares the envelope (`supply + amount <= total`) as a checked invariant: cumulative draws can never exceed the committed grant. Structural shape match, not an overflow proof."
        },
        {
          id: "non_negative",
          label: "Non-negative amounts",
          invariant: "non_negative_amount",
          desc: "Declares `amount >= 0` as a checked invariant on every withdrawal."
        },
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that only the committed recipient key may withdraw — enforced against committed state, never a caller argument."
        }
      ]
    },
    {
      id: "subscription",
      label: "Subscription",
      defaultName: "MySubscription",
      desc: "Recurring pull-payment: a committed provider pulls a fixed fee from a prepaid balance, but no more than once per period.",
      source: "library/finance/subscription/Subscription.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that only the committed provider key may charge — the billing authority cannot be forged or redirected."
        },
        {
          id: "temporal_guard",
          label: "Enforced rate limit",
          invariant: "temporal_guard",
          desc: "Declares the cadence gate (`now_bucket >= last_charged + period`) as a checked invariant: an edit that permits charging faster than once per period fails the checker. Structural shape match, not a wall-clock proof."
        }
      ]
    },
    {
      id: "token",
      label: "Token (single-holder supply + transfer)",
      defaultName: "MyToken",
      desc: "Single-instance token covenant: a committed holder spends from `balance`, a committed minter grows `supply` (the mint path is exempt from value conservation by the mint* naming convention). One covenant instance models ONE holder leg — a starting point, not an account-ledger token.",
      source: "examples SimpleToken/PausableToken + library/finance/kyc-transfer/KycGatedTransfer.portrait",
      features: [
        {
          id: "bounded_supply",
          label: "Enforced supply cap",
          invariant: "bounded_supply",
          desc: "Adds a committed `total` field and requires `supply + amount <= total` on the mint path, enforced by the checker: cumulative mints can never exceed the cap. Structural shape match, not an overflow proof."
        },
        {
          id: "pausable",
          label: "Pausable transfers",
          invariant: "requires paused == 0 guard",
          desc: "Adds a committed `paused` flag (0/1 int, PausableToken shape): transfers require `paused == 0`, and a minter-authorised `set_paused` path flips the flag. A plain committed-flag guard, not a named invariant."
        },
        {
          id: "compliance_gate",
          label: "Committed allow-flag gate",
          invariant: "requires allowed == 1 guard",
          desc: "Adds a committed `allowed` flag (KycGatedTransfer shape): transfers require `allowed == 1`. A structural gate on a committed flag set at instantiation — NOT identity verification and NOT regulatory compliance."
        },
        {
          id: "non_negative",
          label: "Non-negative amounts",
          invariant: "non_negative_amount",
          desc: "Declares `amount >= 0` as a checked invariant on the mint path (the transfer path always carries the same `value >= 0` guard in its body)."
        }
      ]
    },
    {
      id: "vprog",
      label: "Cross-layer (vProg) instrument",
      defaultName: "MyInstrument",
      desc: "CROSS-LAYER PATTERN, not a plain covenant: an on-chain settlement covenant (committed owner auth + seq + state root) PAIRED with an off-chain vProg companion that `portrait atelier-build` lowers to a RISC Zero guest SKELETON. The guest's heavy predicate is a developer-authored stub (returns true by default) — the substantive off-chain claim is yours to write. Generating and compiling this is NOT a live settlement: settling needs the separate prover harness, testnet-only.",
      source: "library/vprog ComplianceCredential / BatchRollup / ProofOfReserves (CSCI shape)",
      /* vProg-specific on-ramp: check gates the covenant side; atelier-build
         emits the guest skeleton. `portrait prove`/`ship` are not the vProg
         on-ramp, so the header swaps in the real pair. */
      verify: [
        "portrait check {file}",
        "portrait atelier-build {file}   # emits the RISC Zero guest SKELETON"
      ],
      headerNotes: [
        "CROSS-LAYER (vProg) PATTERN — what this file is and is NOT:",
        "· It PAIRS an on-chain settlement covenant (`settle`) with an off-chain",
        "  companion (`predicate`, no #[covenant] attribute) that",
        "  `portrait atelier-build` lowers to a RISC Zero guest SKELETON.",
        "· The guest's heavy predicate is a DEVELOPER-AUTHORED stub (it returns",
        "  true by default). The substantive off-chain claim is YOURS to write;",
        "  nothing about it is proven until you author it.",
        "· Generating / compiling this file is NOT a live settlement. Settling a",
        "  proof on chain needs the separate prover harness, and is testnet-only."
      ],
      features: [
        {
          id: "batch_advance",
          label: "Batch settlement (seq += N)",
          invariant: "requires batch_count >= 1 guard",
          desc: "BatchRollup shape: one settlement adopts the claimed next root for a whole batch (nothing about that root is proven until you author the guest predicate) — `batch_count >= 1` is required and `seq` advances by the batch size, so `monotonic_seq` is NOT declared. Off: single-step CSCI shape (`seq + 1`) with `invariant monotonic_seq` declared."
        },
        {
          id: "owner_auth",
          label: "Owner authorisation (declared)",
          invariant: "authorized",
          desc: "Declares committed-owner authorisation as a checked invariant. The settle path always checkSigs the COMMITTED owner key (never a caller argument); this makes `portrait check` reject an edit that drops it."
        },
        {
          id: "epoch_field",
          label: "Epoch counter",
          invariant: "epoch + 1 step (ProofOfReserves shape)",
          desc: "Adds a committed `epoch` field that advances by exactly one per settlement (ProofOfReserves shape): one attestation per reporting period, carried into the state the guest hashes. A structural step in the body, not a named invariant."
        }
      ]
    },
    {
      id: "htlc",
      label: "Hash-time-locked contract (HTLC)",
      defaultName: "MyHtlc",
      desc: "Custody covenant for a reveal-XOR-timeout swap: the recipient claims by revealing a preimage whose blake2b digest matches a committed hashlock, XOR the sender refunds after a deadline. One-shot settled flag prevents double-spend across the two paths. The hashlock is a real on-chain blake2b digest lock (`blake2b(preimage) == hashlock`), NOT a committed-value equality.",
      source: "library/finance/htlc/Htlc.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that claim/refund authority is checked against the COMMITTED recipient/sender keys — a transition without a committed-key checkSig is rejected. The digest lock itself is always in the claim body regardless of this toggle."
        },
        {
          id: "temporal_guard",
          label: "Enforced refund deadline",
          invariant: "temporal_guard",
          desc: "Declares the refund gate (`now_bucket >= deadline`) as a checked invariant. Structural shape match on the guard, not a wall-clock proof — `now_bucket` is caller-asserted and coarse."
        }
      ]
    },
    {
      id: "royalty-split",
      label: "Royalty split (N-payee fan-out)",
      defaultName: "MyRoyaltySplit",
      desc: "A single pooled income leg distributed across three payee legs in one transition. The core mechanism is conservation_split: the combined amount leaving the pool must equal the sum arriving in the payee legs. This is structural N-field additive-delta cancellation, NOT an SMT conservation proof — it does not reason about the numeric values and models an INTERNAL split between the covenant's own legs (not an external payout).",
      source: "library/finance/royalty-split/RoyaltySplit.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that distribution authority is checked against the COMMITTED distributor key — a transition without a committed-key checkSig is rejected. The distribute body always checkSigs the committed distributor regardless of this toggle."
        }
      ]
    },
    {
      id: "collateral-loan",
      label: "Collateral vault (CDP)",
      defaultName: "MyCollateralVault",
      desc: "A single-owner over-collateralised debt position: deposit collateral, borrow against it, repay. The borrow path is gated by a committed-integer ratio comparison (`collateral >= (debt + amount) * min_ratio`). That guard is a committed-integer comparison with NO oracle / price feed, NO fractional ratios (integer multiply only), and NO liquidation-safety proof — it is always present in the borrow body.",
      source: "library/finance/collateral-vault/CollateralVault.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that every deposit/borrow/repay is authorised against the COMMITTED owner key — a mutation without a committed-key checkSig is rejected."
        },
        {
          id: "non_negative",
          label: "Non-negative amounts",
          invariant: "non_negative_amount",
          desc: "Declares `amount >= 0` as a checked invariant on every transition that takes an int `amount` — a future edit that drops a bound fails the checker."
        }
      ]
    },
    {
      id: "allowance",
      label: "Token allowance (approve / transferFrom)",
      defaultName: "MyTokenAllowance",
      desc: "The ERC-20 approve / transferFrom delegated-spend shape: an owner grants a single spender a capped allowance to pull from the owner's balance. `approve` (owner path) resets the cap; `transfer_from` (spender path) is gated on both the committed allowance and balance, and debits both by the same amount. Models a SINGLE owner→spender pair (a second spender is a second covenant instance), not an allowances mapping.",
      source: "library/finance/token-allowance/TokenAllowance.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that each path authorises against its COMMITTED key (approve→owner, transfer_from→spender) — a transition without a committed-key checkSig is rejected."
        },
        {
          id: "non_negative",
          label: "Non-negative amounts",
          invariant: "non_negative_amount",
          desc: "Declares `amount >= 0` on the pulled/approved amount as a checked invariant — a future edit that drops the bound fails the checker."
        }
      ]
    },
    {
      id: "social-recovery",
      label: "Social recovery (2-of-3 guardians)",
      defaultName: "MySocialRecovery",
      desc: "A guardian-based account-recovery covenant: the owner key can be rotated without the owner when any two of three committed guardians cooperate. `propose_recovery` stages a pending owner (arm), `finalize` ratifies it (rotate + disarm). The 2-of-3 is a STRUCTURAL count of distinct committed guardian keys signed (a disjunction of conjunctive pairs), NOT an identity or social-graph system and NOT a proof the boolean combination is a true k-of-n threshold.",
      source: "library/governance/social-recovery/SocialRecovery.portrait",
      features: [
        {
          id: "committed_auth",
          label: "Committed-key authorisation",
          invariant: "authorized",
          desc: "Declares that both recovery transitions authorise against COMMITTED guardian keys carried in state — never a caller-supplied pubkey. A transition without a committed-key checkSig is rejected."
        },
        {
          id: "multisig_threshold",
          label: "Enforced 2-of-3 guardian threshold",
          invariant: "multisig_threshold",
          desc: "Declares that each transition authorises via at least 2 distinct committed guardian-key checkSig operands. A future edit down to a single committed-key checkSig is rejected. Structural count of distinct committed keys signed, not a true-threshold proof."
        }
      ]
    }
  ];

  /* ── Helpers ── */

  function patternById(id) {
    for (var i = 0; i < PATTERNS.length; i++) {
      if (PATTERNS[i].id === id) return PATTERNS[i];
    }
    return null;
  }

  /* Sanitize a user-supplied app name into a valid Portrait identifier. */
  function sanitizeName(name, fallback) {
    var s = String(name == null ? "" : name).replace(/[^A-Za-z0-9_]/g, "");
    if (!/^[A-Za-z_]/.test(s)) s = "";
    return s || fallback;
  }

  function toSet(features) {
    var set = {};
    if (Array.isArray(features)) {
      for (var i = 0; i < features.length; i++) set[features[i]] = true;
    } else if (features && typeof features === "object") {
      for (var k in features) { if (features[k]) set[k] = true; }
    }
    return set;
  }

  function header(name, pat, on) {
    var toggles = [];
    for (var i = 0; i < pat.features.length; i++) {
      if (on[pat.features[i].id]) toggles.push(pat.features[i].id);
    }
    /* Per-pattern verify commands ({file} placeholder) and extra honesty
       notes; defaults keep the original 7 patterns byte-identical. */
    var verify = pat.verify || [
      "portrait check {file}",
      "portrait prove {file}",
      "portrait ship  {file}"
    ];
    var lines = [
      "pragma portrait ^0.1.0;",
      "",
      "// " + name + " — generated by the kaspa-compliance-patterns wizard.",
      "// Pattern: " + pat.label.toLowerCase() + " (derived from " + pat.source + ")",
      "// Toggles: " + (toggles.length ? toggles.join(", ") : "(none)"),
      "//",
      "// STARTING POINT ONLY — pre-production, unaudited, testnet-only.",
      "// This file compiles and passes `portrait check`; that is a structural",
      "// gate, NOT an audit and NOT a security guarantee. Verify it yourself:"
    ];
    for (var j = 0; j < verify.length; j++) {
      lines.push("//   " + verify[j].replace("{file}", name + ".portrait"));
    }
    if (pat.headerNotes && pat.headerNotes.length) {
      lines.push("//");
      for (var k = 0; k < pat.headerNotes.length; k++) {
        lines.push("// " + pat.headerNotes[k]);
      }
    }
    lines.push("");
    return lines.join("\n");
  }

  /* ── Per-pattern generators ── */

  function genCounter(name, on) {
    var strict = !!on.strict_sequence;
    var auth = !!on.owner_auth;
    var s;
    if (!strict && !auth) {
      /* Exactly the known-good `portrait new --template counter` shape. */
      s = [
        "app " + name + " {",
        "  role counter {",
        "    param int start;",
        "    state { int value; }",
        "",
        "    #[covenant(mode = transition)]",
        "    entrypoint function bump(int delta) : (int value) {",
        "      return value + delta;",
        "    }",
        "  }",
        "",
        "  lifecycle { live -> live via counter.bump; }",
        "  invariant no_undeclared_state;",
        "}"
      ];
      return s.join("\n") + "\n";
    }
    /* Structured (object-return) counter: seq field, optional committed owner. */
    var args = [];
    if (auth) args.push("sig auth");
    if (!strict) args.push("int delta");
    var retFields = (auth ? "pubkey owner, " : "") + "int seq";
    s = ["app " + name + " {", "  role counter {"];
    if (auth) s.push("    param pubkey owner;   // committed operator key (bump authority)");
    s.push("    param int    seq;     // counter value (genesis = 0)");
    s.push("");
    s.push("    state {");
    if (auth) s.push("      pubkey owner;");
    s.push("      int    seq;");
    s.push("    }");
    s.push("");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function bump(" + args.join(", ") + ") : (" + retFields + ") {");
    if (auth) s.push("      requires checkSig(auth, owner);   // committed-owner authorisation");
    s.push("      return " + name + " {");
    if (auth) s.push("        owner: owner,");
    s.push("        seq:   " + (strict ? "seq + 1" : "seq + delta"));
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle { live -> live via counter.bump; }");
    s.push("");
    if (auth) s.push("  invariant authorized;");
    if (strict) s.push("  invariant monotonic_seq;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genVault(name, on) {
    var claw = !!on.recovery_clawback;
    var fields = claw
      ? "pubkey owner, pubkey recovery, int unlock_bucket, int released"
      : "pubkey owner, int unlock_bucket, int released";
    var s = ["app " + name + " {", "  role vault {"];
    s.push("    param pubkey owner;          // hot key permitted to release after the gate");
    if (claw) s.push("    param pubkey recovery;       // cold clawback key (break-glass)");
    s.push("    param int    unlock_bucket;  // coarse time bucket at/after which release is allowed");
    s.push("    param int    released;       // one-shot spent flag (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey owner;");
    if (claw) s.push("      pubkey recovery;");
    s.push("      int    unlock_bucket;");
    s.push("      int    released;");
    s.push("    }");
    s.push("");
    s.push("    // Owner releases the funds once the temporal gate has opened.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function release(");
    s.push("      sig auth,");
    s.push("      int now_bucket");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);        // only the owner may release");
    s.push("      requires now_bucket >= unlock_bucket;  // temporal gate has opened");
    s.push("      requires released == 0;                // one-shot: not already spent");
    s.push("      return " + name + " {");
    s.push("        owner:         owner,");
    if (claw) s.push("        recovery:      recovery,");
    s.push("        unlock_bucket: unlock_bucket,");
    s.push("        released:      1");
    s.push("      };");
    s.push("    }");
    if (claw) {
      s.push("");
      s.push("    // Recovery key claws the funds back (break-glass), before release.");
      s.push("    #[covenant(mode = transition)]");
      s.push("    entrypoint function claw(");
      s.push("      sig auth");
      s.push("    ) : (" + fields + ") {");
      s.push("      requires checkSig(auth, recovery);   // only the recovery key may claw");
      s.push("      requires released == 0;              // one-shot: not already spent");
      s.push("      return " + name + " {");
      s.push("        owner:         owner,");
      s.push("        recovery:      recovery,");
      s.push("        unlock_bucket: unlock_bucket,");
      s.push("        released:      1");
      s.push("      };");
      s.push("    }");
    }
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via vault.release;");
    if (claw) s.push("    live -> live via vault.claw;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.temporal_guard) s.push("  invariant temporal_guard;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genEscrow(name, on) {
    var fields = "pubkey buyer, pubkey seller, coin amount, int deadline, int settled";
    var s = ["app " + name + " {", "  role escrow {"];
    s.push("    param pubkey buyer;     // funds the escrow (refund authority)");
    s.push("    param pubkey seller;    // delivers (release authority)");
    s.push("    param coin   amount;    // value locked (coin: strictly conserved)");
    s.push("    param int    deadline;  // coarse time bucket at/after which refund is allowed");
    s.push("    param int    settled;   // one-shot spent flag (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey buyer;");
    s.push("      pubkey seller;");
    s.push("      coin   amount;");
    s.push("      int    deadline;");
    s.push("      int    settled;");
    s.push("    }");
    s.push("");
    s.push("    // Happy path: the committed seller settles the escrow.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function release(");
    s.push("      sig auth");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, seller);");
    s.push("      requires settled == 0;");
    s.push("      return " + name + " {");
    s.push("        buyer:    buyer,");
    s.push("        seller:   seller,");
    s.push("        amount:   amount,");
    s.push("        deadline: deadline,");
    s.push("        settled:  1");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Timeout path: the committed buyer claws back after the deadline.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function refund(");
    s.push("      sig auth,");
    s.push("      int now_bucket");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, buyer);");
    s.push("      requires now_bucket >= deadline;");
    s.push("      requires settled == 0;");
    s.push("      return " + name + " {");
    s.push("        buyer:    buyer,");
    s.push("        seller:   seller,");
    s.push("        amount:   amount,");
    s.push("        deadline: deadline,");
    s.push("        settled:  1");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via escrow.release;");
    s.push("    live -> live via escrow.refund;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.temporal_guard) s.push("  invariant temporal_guard;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genTreasury(name, on) {
    var cap = !!on.spending_cap;
    var fields = cap
      ? "pubkey signer_a, pubkey signer_b, int balance, int limit"
      : "pubkey signer_a, pubkey signer_b, int balance";
    var s = ["app " + name + " {", "  role treasury {"];
    s.push("    param pubkey signer_a;   // first committed signer");
    s.push("    param pubkey signer_b;   // second committed signer");
    s.push("    param int    balance;    // treasury balance (value-conserved)");
    if (cap) s.push("    param int    limit;      // committed per-transaction spending cap");
    s.push("");
    s.push("    state {");
    s.push("      pubkey signer_a;");
    s.push("      pubkey signer_b;");
    s.push("      int    balance;");
    if (cap) s.push("      int    limit;");
    s.push("    }");
    s.push("");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function spend(");
    s.push("      sig auth_a,");
    s.push("      sig auth_b,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth_a, signer_a);");
    s.push("      requires checkSig(auth_b, signer_b);");
    s.push("      requires amount >= 0;");
    if (cap) s.push("      requires amount <= limit;    // per-transaction spending cap");
    s.push("      requires amount <= balance;");
    s.push("      return " + name + " {");
    s.push("        signer_a: signer_a,");
    s.push("        signer_b: signer_b,");
    s.push("        balance:  balance - amount" + (cap ? "," : ""));
    if (cap) s.push("        limit:    limit");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via treasury.spend;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.non_negative) s.push("  invariant non_negative_amount;");
    if (cap) s.push("  invariant spending_cap;");
    if (on.multisig_threshold) s.push("  invariant multisig_threshold;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genVesting(name, on) {
    var fields = "pubkey recipient, int total, int start, int duration, int supply";
    var s = ["app " + name + " {", "  role stream {"];
    s.push("    param pubkey recipient;   // committed payee — the only withdrawal authority");
    s.push("    param int    total;       // full grant ceiling (constant; carried unchanged)");
    s.push("    param int    start;       // schedule anchor (coarse time bucket)");
    s.push("    param int    duration;    // length of the vesting window");
    s.push("    param int    supply;      // cumulative amount withdrawn (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey recipient;");
    s.push("      int    total;");
    s.push("      int    start;");
    s.push("      int    duration;");
    s.push("      int    supply;");
    s.push("    }");
    s.push("");
    s.push("    // The recipient withdraws a portion of the grant.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function withdraw(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, recipient);   // committed-recipient authorisation");
    s.push("      requires amount >= 0;                 // non-negative withdrawal");
    s.push("      requires supply + amount <= total;    // cumulative draw never exceeds grant");
    s.push("      return " + name + " {");
    s.push("        recipient: recipient,");
    s.push("        total:     total,");
    s.push("        start:     start,");
    s.push("        duration:  duration,");
    s.push("        supply:    supply + amount");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via stream.withdraw;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.non_negative) s.push("  invariant non_negative_amount;");
    if (on.bounded_supply) s.push("  invariant bounded_supply;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genSubscription(name, on) {
    var fields = "pubkey provider, pubkey subscriber, int amount_per_period, int period, int last_charged, int balance";
    var s = ["app " + name + " {", "  role subscription {"];
    s.push("    param pubkey provider;          // committed merchant key (billing authority)");
    s.push("    param pubkey subscriber;        // committed customer key (carried; for reference)");
    s.push("    param int    amount_per_period; // fixed fee pulled per period");
    s.push("    param int    period;            // rate limit: minimum buckets between charges");
    s.push("    param int    last_charged;      // coarse time bucket of the last charge");
    s.push("    param int    balance;           // prepaid balance (value-conserved)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey provider;");
    s.push("      pubkey subscriber;");
    s.push("      int    amount_per_period;");
    s.push("      int    period;");
    s.push("      int    last_charged;");
    s.push("      int    balance;");
    s.push("    }");
    s.push("");
    s.push("    // Billing path: the committed provider pulls one period's fee.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function charge(");
    s.push("      sig auth,");
    s.push("      int now_bucket");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, provider);             // only the committed provider may charge");
    s.push("      requires now_bucket >= last_charged + period;  // rate limit: one charge per period");
    s.push("      requires amount_per_period >= 0;               // non-negative fee");
    s.push("      requires amount_per_period <= balance;         // cannot overdraw the subscription");
    s.push("      return " + name + " {");
    s.push("        provider:          provider,");
    s.push("        subscriber:        subscriber,");
    s.push("        amount_per_period: amount_per_period,");
    s.push("        period:            period,");
    s.push("        last_charged:      now_bucket,");
    s.push("        balance:           balance - amount_per_period");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via subscription.charge;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.temporal_guard) s.push("  invariant temporal_guard;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genToken(name, on) {
    var bounded = !!on.bounded_supply;
    var pause = !!on.pausable;
    var gate = !!on.compliance_gate;
    /* State-field order (constructor param per state field, in field order):
       holder, minter, [allowed], [paused], [total], supply, balance. */
    var fields = "pubkey holder, pubkey minter"
      + (gate ? ", int allowed" : "")
      + (pause ? ", int paused" : "")
      + (bounded ? ", int total" : "")
      + ", int supply, int balance";
    function carried(s, pausedValue) {
      /* Shared return prologue (state-field order): keys + flags carried
         unchanged, except `paused` which the set_paused path replaces. */
      s.push("        holder:  holder,");
      s.push("        minter:  minter,");
      if (gate) s.push("        allowed: allowed,");
      if (pause) s.push("        paused:  " + pausedValue + ",");
      if (bounded) s.push("        total:   total,");
    }
    var s = ["app " + name + " {", "  role token {"];
    s.push("    param pubkey holder;    // committed transfer authority (this instance's holder)");
    s.push("    param pubkey minter;    // committed mint authority" + (pause ? " (also flips the paused flag)" : ""));
    if (gate) s.push("    param int    allowed;   // committed allow-flag (0 = blocked, 1 = allowed; set at instantiation)");
    if (pause) s.push("    param int    paused;    // committed paused-flag (0 = live, 1 = paused; genesis = 0)");
    if (bounded) s.push("    param int    total;     // committed supply cap (constant; carried unchanged)");
    s.push("    param int    supply;    // cumulative minted supply");
    s.push("    param int    balance;   // balance held by this covenant instance");
    s.push("");
    s.push("    state {");
    s.push("      pubkey holder;");
    s.push("      pubkey minter;");
    if (gate) s.push("      int    allowed;");
    if (pause) s.push("      int    paused;");
    if (bounded) s.push("      int    total;");
    s.push("      int    supply;");
    s.push("      int    balance;");
    s.push("    }");
    s.push("");
    s.push("    // The committed holder moves `value` out of this instance's balance.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function transfer(");
    s.push("      sig auth,");
    s.push("      int value");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, holder);   // only the committed holder may transfer");
    if (gate) s.push("      requires allowed == 1;             // committed allow-flag gate");
    if (pause) s.push("      requires paused == 0;              // committed paused-flag guard");
    s.push("      requires value >= 0;               // non-negative transfer");
    s.push("      requires value <= balance;         // cannot overdraw the balance");
    s.push("      return " + name + " {");
    carried(s, "paused");
    s.push("        supply:  supply,");
    s.push("        balance: balance - value");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // The committed minter grows the supply (mint* naming convention:");
    s.push("    // exempt from value_conserved as an authorised supply change).");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function mint(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, minter);   // only the committed minter may mint");
    s.push("      requires amount >= 0;              // non-negative mint");
    if (bounded) s.push("      requires supply + amount <= total; // cumulative mints never exceed the cap");
    s.push("      return " + name + " {");
    carried(s, "paused");
    s.push("        supply:  supply + amount,");
    s.push("        balance: balance + amount");
    s.push("      };");
    s.push("    }");
    if (pause) {
      s.push("");
      s.push("    // The committed minter flips the paused flag (0 = live, 1 = paused).");
      s.push("    #[covenant(mode = transition)]");
      s.push("    entrypoint function set_paused(");
      s.push("      sig auth,");
      s.push("      int flag");
      s.push("    ) : (" + fields + ") {");
      s.push("      requires checkSig(auth, minter);   // only the committed minter may pause/unpause");
      s.push("      return " + name + " {");
      carried(s, "flag");
      s.push("        supply:  supply,");
      s.push("        balance: balance");
      s.push("      };");
      s.push("    }");
    }
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via token.transfer;");
    s.push("    live -> live via token.mint;");
    if (pause) s.push("    live -> live via token.set_paused;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (bounded) s.push("  invariant bounded_supply;");
    if (on.non_negative) s.push("  invariant non_negative_amount;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genVprog(name, on) {
    var batch = !!on.batch_advance;
    var epoch = !!on.epoch_field;
    var fields = "pubkey owner"
      + (epoch ? ", int epoch" : "")
      + ", bytes32 state_root, int seq";
    /* Shared return body: the vProg companion MIRRORS `settle` (CSCI shape),
       so the emitted guest hashes the NEW state into the journal. */
    function retBody(s) {
      s.push("      return " + name + " {");
      s.push("        owner:      owner,             // owner key carried unchanged");
      if (epoch) s.push("        epoch:      epoch + 1,         // reporting epoch advances by one");
      s.push("        state_root: next_root,         // adopt the CLAIMED next state root (unproven until you author the guest predicate)");
      if (batch) s.push("        seq:        seq + batch_count  // sequence advances by the batch size");
      else s.push("        seq:        seq + 1            // sequence advances by exactly one");
      s.push("      };");
    }
    var s = ["app " + name + " {", "  role instrument {"];
    s.push("    param pubkey  owner;       // committed owner key (the settle authority)");
    if (epoch) s.push("    param int     epoch;       // monotonic reporting epoch (genesis = 0)");
    s.push("    param bytes32 state_root;  // committed state root the vProg advances");
    s.push("    param int     seq;         // monotonic sequence number (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey  owner;");
    if (epoch) s.push("      int     epoch;");
    s.push("      bytes32 state_root;");
    s.push("      int     seq;");
    s.push("    }");
    s.push("");
    s.push("    // ON-CHAIN settlement covenant: records ONE " + (batch ? "batch settlement" : "settled transition") + ".");
    s.push("    // The companion's presence (`has_vprog`) is what causes the covenant-id");
    s.push("    // binding require() to be emitted into this path by the compiler.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function settle(");
    s.push("      sig auth,");
    s.push("      bytes32 next_root" + (batch ? "," : ""));
    if (batch) s.push("      int batch_count");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);    // committed-owner authorisation");
    if (batch) s.push("      requires batch_count >= 1;         // a batch advances the instrument forward");
    retBody(s);
    s.push("    }");
    s.push("");
    s.push("    // OFF-CHAIN vProg companion: NO #[covenant] attribute → NonCovenant →");
    s.push("    // `portrait atelier-build` emits a RISC Zero guest SKELETON for it.");
    s.push("    // The guest's heavy predicate is a DEVELOPER-AUTHORED stub (returns");
    s.push("    // true by default): the substantive claim behind `next_root` is YOURS");
    s.push("    // to write. Nothing about it is proven until you author it.");
    s.push("    entrypoint function predicate(bytes32 next_root" + (batch ? ", int batch_count" : "") + ") {");
    if (batch) s.push("      requires batch_count >= 1;         // assertable in-guest");
    retBody(s);
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle { live -> live via instrument.settle; }");
    s.push("");
    if (on.owner_auth) s.push("  invariant authorized;");
    if (!batch) s.push("  invariant monotonic_seq;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genHtlc(name, on) {
    var fields = "pubkey sender, pubkey recipient, bytes32 hashlock, int deadline, int settled";
    var s = ["app " + name + " {", "  role htlc {"];
    s.push("    param pubkey  sender;     // funds the HTLC (refund authority)");
    s.push("    param pubkey  recipient;  // may claim by revealing the preimage");
    s.push("    param bytes32 hashlock;   // committed blake2b digest; claim must reveal a matching preimage");
    s.push("    param int     deadline;   // coarse time bucket at/after which refund is allowed");
    s.push("    param int     settled;    // one-shot spent flag (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey  sender;");
    s.push("      pubkey  recipient;");
    s.push("      bytes32 hashlock;");
    s.push("      int     deadline;");
    s.push("      int     settled;");
    s.push("    }");
    s.push("");
    s.push("    // Hashlock path: the recipient claims by revealing the preimage.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function claim(");
    s.push("      sig auth,");
    s.push("      bytes32 preimage");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, recipient);      // only the committed recipient may claim");
    s.push("      requires blake2b(preimage) == hashlock;  // TRUE hashlock: digest(preimage) == committed hashlock");
    s.push("      requires settled == 0;                   // one-shot: not already settled");
    s.push("      return " + name + " {");
    s.push("        sender:    sender,");
    s.push("        recipient: recipient,");
    s.push("        hashlock:  hashlock,");
    s.push("        deadline:  deadline,");
    s.push("        settled:   1");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Timeout path: the sender claws the funds back after the deadline.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function refund(");
    s.push("      sig auth,");
    s.push("      int now_bucket");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, sender);   // only the committed sender may refund");
    s.push("      requires now_bucket >= deadline;   // temporal gate: deadline has passed");
    s.push("      requires settled == 0;             // one-shot: not already settled");
    s.push("      return " + name + " {");
    s.push("        sender:    sender,");
    s.push("        recipient: recipient,");
    s.push("        hashlock:  hashlock,");
    s.push("        deadline:  deadline,");
    s.push("        settled:   1");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via htlc.claim;");
    s.push("    live -> live via htlc.refund;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.temporal_guard) s.push("  invariant temporal_guard;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genRoyaltySplit(name, on) {
    var fields = "int income_balance, int payee_a_balance, int payee_b_balance, int payee_c_balance, pubkey distributor";
    var s = ["app " + name + " {", "  role pool {"];
    s.push("    param int    income_balance;    // pooled source leg (value-bearing)");
    s.push("    param int    payee_a_balance;   // royalty recipient one (value-bearing)");
    s.push("    param int    payee_b_balance;   // royalty recipient two (value-bearing)");
    s.push("    param int    payee_c_balance;   // royalty recipient three (value-bearing)");
    s.push("    param pubkey distributor;       // committed distributor — sole distribution authority");
    s.push("");
    s.push("    state {");
    s.push("      int    income_balance;");
    s.push("      int    payee_a_balance;");
    s.push("      int    payee_b_balance;");
    s.push("      int    payee_c_balance;");
    s.push("      pubkey distributor;");
    s.push("    }");
    s.push("");
    s.push("    // Distribute `a + b + c` out of the pooled income leg: a to payee a,");
    s.push("    // b to payee b, c to payee c. The four deltas net to zero.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function distribute(");
    s.push("      sig auth,");
    s.push("      int a,");
    s.push("      int b,");
    s.push("      int c");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, distributor);   // only the committed distributor may distribute");
    s.push("      requires a >= 0;                         // non-negative royalty legs");
    s.push("      requires b >= 0;");
    s.push("      requires c >= 0;");
    s.push("      requires a + b + c <= income_balance;    // cannot overdraw the pooled income");
    s.push("      return " + name + " {");
    s.push("        income_balance:  income_balance - (a + b + c),  // pooled income decreases by the combined term");
    s.push("        payee_a_balance: payee_a_balance + a,           // payee a gains a");
    s.push("        payee_b_balance: payee_b_balance + b,           // payee b gains b");
    s.push("        payee_c_balance: payee_c_balance + c,           // payee c gains c");
    s.push("        distributor:     distributor                    // distributor carried unchanged");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via pool.distribute;");
    s.push("  }");
    s.push("");
    s.push("  invariant conservation_split;");
    if (on.committed_auth) s.push("  invariant authorized;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genCollateralLoan(name, on) {
    var fields = "pubkey owner, int collateral, int debt, int min_ratio";
    var s = ["app " + name + " {", "  role vault {"];
    s.push("    param pubkey owner;        // committed owner key (sole mutation authority)");
    s.push("    param int    collateral;   // value-bearing collateral leg");
    s.push("    param int    debt;         // value-bearing debt leg");
    s.push("    param int    min_ratio;    // committed collateralisation multiplier");
    s.push("");
    s.push("    state {");
    s.push("      pubkey owner;");
    s.push("      int    collateral;");
    s.push("      int    debt;");
    s.push("      int    min_ratio;");
    s.push("    }");
    s.push("");
    s.push("    // Deposit collateral. Owner-authorised; the collateral leg grows.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function deposit(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);   // committed-owner authorisation");
    s.push("      requires amount >= 0;             // non-negative deposit");
    s.push("      return " + name + " {");
    s.push("        owner:      owner,");
    s.push("        collateral: collateral + amount,");
    s.push("        debt:       debt,");
    s.push("        min_ratio:  min_ratio");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Draw debt against the committed collateral. The structural ratio guard");
    s.push("    // must hold on the POST-borrow debt: (debt + amount) * min_ratio <= collateral.");
    s.push("    // Integer multiply, no oracle, no fractions — see the honest-scope note.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function borrow(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);                       // committed-owner authorisation");
    s.push("      requires amount >= 0;                                 // non-negative draw");
    s.push("      requires collateral >= (debt + amount) * min_ratio;   // structural collateralisation guard (post-borrow)");
    s.push("      return " + name + " {");
    s.push("        owner:      owner,");
    s.push("        collateral: collateral,");
    s.push("        debt:       debt + amount,");
    s.push("        min_ratio:  min_ratio");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Repay debt. Owner-authorised; the debt leg shrinks.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function repay(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);   // committed-owner authorisation");
    s.push("      requires amount >= 0;             // non-negative repayment");
    s.push("      requires amount <= debt;          // cannot repay more than is owed");
    s.push("      return " + name + " {");
    s.push("        owner:      owner,");
    s.push("        collateral: collateral,");
    s.push("        debt:       debt - amount,");
    s.push("        min_ratio:  min_ratio");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via vault.deposit;");
    s.push("    live -> live via vault.borrow;");
    s.push("    live -> live via vault.repay;");
    s.push("  }");
    s.push("");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.non_negative) s.push("  invariant non_negative_amount;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genAllowance(name, on) {
    var fields = "pubkey owner, pubkey spender, int allowance, int balance";
    var s = ["app " + name + " {", "  role allowance {"];
    s.push("    param pubkey owner;       // committed owner key (approve authority; holds the balance)");
    s.push("    param pubkey spender;     // committed spender key (transfer_from authority; the grantee)");
    s.push("    param int    allowance;   // committed delegated-spend cap (accounting ledger)");
    s.push("    param int    balance;     // owner's balance (value-conserved by field name)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey owner;");
    s.push("      pubkey spender;");
    s.push("      int    allowance;");
    s.push("      int    balance;");
    s.push("    }");
    s.push("");
    s.push("    // Owner path (ERC-20 approve): the owner (re)sets the spender's cap.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function approve(");
    s.push("      sig auth,");
    s.push("      int new_allowance");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, owner);   // only the committed owner may approve");
    s.push("      requires new_allowance >= 0;      // a cap is non-negative");
    s.push("      return " + name + " {");
    s.push("        owner:     owner,");
    s.push("        spender:   spender,");
    s.push("        allowance: new_allowance,");
    s.push("        balance:   balance");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Spender path (ERC-20 transferFrom): the spender pulls `amount` from the");
    s.push("    // owner's balance, gated on BOTH committed caps; debits BOTH by the same amount.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function transfer_from(");
    s.push("      sig auth,");
    s.push("      int amount");
    s.push("    ) : (" + fields + ") {");
    s.push("      requires checkSig(auth, spender);   // only the committed spender may pull");
    s.push("      requires amount >= 0;               // non-negative pull");
    s.push("      requires amount <= allowance;       // may not exceed the committed grant");
    s.push("      requires amount <= balance;         // may not overdraw the owner's balance");
    s.push("      return " + name + " {");
    s.push("        owner:     owner,");
    s.push("        spender:   spender,");
    s.push("        allowance: allowance - amount,    // grant debited as it is spent");
    s.push("        balance:   balance - amount       // single additive subtraction (value conserved)");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via allowance.approve;");
    s.push("    live -> live via allowance.transfer_from;");
    s.push("  }");
    s.push("");
    s.push("  invariant value_conserved;");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.non_negative) s.push("  invariant non_negative_amount;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  function genSocialRecovery(name, on) {
    var fields = "pubkey owner, pubkey guardian_a, pubkey guardian_b, pubkey guardian_c, pubkey pending_owner, int recovering";
    function quorum(s) {
      s.push("      requires checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_b)");
      s.push("            || checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_c)");
      s.push("            || checkSig(auth_x, guardian_b) && checkSig(auth_y, guardian_c);");
    }
    var s = ["app " + name + " {", "  role account {"];
    s.push("    param pubkey owner;          // current account owner (rotated by recovery)");
    s.push("    param pubkey guardian_a;     // committed guardian 1 (recovery authority)");
    s.push("    param pubkey guardian_b;     // committed guardian 2 (recovery authority)");
    s.push("    param pubkey guardian_c;     // committed guardian 3 (recovery authority)");
    s.push("    param pubkey pending_owner;  // nominee staged by a proposal (genesis: placeholder)");
    s.push("    param int    recovering;     // arm/disarm flag (genesis = 0)");
    s.push("");
    s.push("    state {");
    s.push("      pubkey owner;");
    s.push("      pubkey guardian_a;");
    s.push("      pubkey guardian_b;");
    s.push("      pubkey guardian_c;");
    s.push("      pubkey pending_owner;");
    s.push("      int    recovering;         // 0 = idle, 1 = recovery armed");
    s.push("    }");
    s.push("");
    s.push("    // Propose a recovery: any 2-of-3 committed guardians nominate a new");
    s.push("    // pending_owner and arm the recovery. `&&` binds tighter than `||`.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function propose_recovery(");
    s.push("      sig    auth_x,");
    s.push("      sig    auth_y,");
    s.push("      pubkey new_owner");
    s.push("    ) : (" + fields + ") {");
    quorum(s);
    s.push("      requires recovering == 0;        // cannot re-arm an in-flight recovery");
    s.push("      return " + name + " {");
    s.push("        owner:         owner,          // owner unchanged until finalize");
    s.push("        guardian_a:    guardian_a,");
    s.push("        guardian_b:    guardian_b,");
    s.push("        guardian_c:    guardian_c,");
    s.push("        pending_owner: new_owner,      // stage the nominee");
    s.push("        recovering:    1               // arm the recovery");
    s.push("      };");
    s.push("    }");
    s.push("");
    s.push("    // Finalize a recovery: the same 2-of-3 threshold confirms the staged");
    s.push("    // rotation. The owner key becomes pending_owner and recovery disarms.");
    s.push("    #[covenant(mode = transition)]");
    s.push("    entrypoint function finalize(");
    s.push("      sig auth_x,");
    s.push("      sig auth_y");
    s.push("    ) : (" + fields + ") {");
    quorum(s);
    s.push("      requires recovering == 1;        // cannot finalize what was never proposed");
    s.push("      return " + name + " {");
    s.push("        owner:         pending_owner,  // rotate owner to the staged nominee");
    s.push("        guardian_a:    guardian_a,");
    s.push("        guardian_b:    guardian_b,");
    s.push("        guardian_c:    guardian_c,");
    s.push("        pending_owner: pending_owner,  // carry nominee (now the owner)");
    s.push("        recovering:    0               // disarm the recovery");
    s.push("      };");
    s.push("    }");
    s.push("  }");
    s.push("");
    s.push("  lifecycle {");
    s.push("    live -> live via account.propose_recovery;");
    s.push("    live -> live via account.finalize;");
    s.push("  }");
    s.push("");
    if (on.committed_auth) s.push("  invariant authorized;");
    if (on.multisig_threshold) s.push("  invariant multisig_threshold;");
    s.push("  invariant no_undeclared_state;");
    s.push("}");
    return s.join("\n") + "\n";
  }

  var GENERATORS = {
    counter: genCounter,
    vault: genVault,
    escrow: genEscrow,
    treasury: genTreasury,
    vesting: genVesting,
    subscription: genSubscription,
    token: genToken,
    vprog: genVprog,
    htlc: genHtlc,
    "royalty-split": genRoyaltySplit,
    "collateral-loan": genCollateralLoan,
    allowance: genAllowance,
    "social-recovery": genSocialRecovery
  };

  /* ── Public API ──
     generate(patternId, features, name) -> String (.portrait source)
       patternId : one of PATTERNS[i].id
       features  : array of feature ids, or {id: bool} map
       name      : app name (sanitized; falls back to the pattern default) */
  function generate(patternId, features, name) {
    var pat = patternById(patternId);
    if (!pat) throw new Error("unknown pattern: " + patternId);
    var on = toSet(features);
    /* Ignore feature ids the pattern does not offer (deterministic output). */
    var valid = {};
    for (var i = 0; i < pat.features.length; i++) {
      var id = pat.features[i].id;
      if (on[id]) valid[id] = true;
    }
    var appName = sanitizeName(name, pat.defaultName);
    return header(appName, pat, valid) + GENERATORS[patternId](appName, valid);
  }

  var api = { generate: generate, PATTERNS: PATTERNS, sanitizeName: sanitizeName };

  if (typeof module !== "undefined" && module.exports) {
    module.exports = api;
  }
  if (typeof window !== "undefined") {
    window.KcpWizard = api;
  }
})();
