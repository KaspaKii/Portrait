//! AST → SMT-LIB term map `⟦·⟧` and the value-bearing field-set predicate.
//!
//! This implements the encoding table of `docs/LENS-M0-ENCODING-SPEC.md` §2.2 for
//! the nodes the M1 conservation VC needs. Soundness corners (`Field`, `Index`,
//! `Call`) are conservative: opaque / uninterpreted, never over-assumed.

use std::collections::BTreeMap;

use portrait_syntax::{BinOp, Expr, Field, Type, UnOp};

/// Encoding failure: a node the conservation VC cannot encode soundly. Surfaced
/// as a `LensError::Unsupported` by the caller — never silently dropped (which
/// could make a VC vacuously discharge), and never a `PROVED`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

/// Which SMT-LIB logic a term family lands in. The whole document uses the least
/// upper bound over the terms it emits (spec §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Logic {
    /// Quantifier-free linear integer arithmetic (the decidable common case).
    QfLia,
    /// Adds uninterpreted functions + arrays (Field/Index/Call/checkSig).
    QfAuflia,
}

impl Logic {
    /// The SMT-LIB `(set-logic ...)` name.
    pub fn name(self) -> &'static str {
        match self {
            Logic::QfLia => "QF_LIA",
            Logic::QfAuflia => "QF_AUFLIA",
        }
    }

    /// Least upper bound (join) of two logics.
    pub fn join(self, other: Logic) -> Logic {
        self.max(other)
    }
}

/// Result sort expected of an encoded expression. Drives the result sort of an
/// uninterpreted `Call` (a `checkSig`-style guard is `Bool`; an arithmetic
/// operand is `Int`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectSort {
    /// Boolean context (e.g. a `require` operand or a logical sub-term).
    Bool,
    /// Integer context (e.g. an arithmetic operand or a conserved value).
    Int,
}

/// A collected uninterpreted-function signature, so the document can emit a
/// `(declare-fun name (argsorts) ret)` line. Without this, z3 rejects the script
/// (→ UNKNOWN, sound but useless); declaring keeps `checkSig` opaque (NOT assumed
/// true) so conservation is provable independent of *who* signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UfSig {
    /// SMT-LIB argument sorts, in order.
    pub args: Vec<String>,
    /// SMT-LIB result sort.
    pub ret: String,
}

/// Context for encoding: maps every in-scope bare name (state field, param, arg)
/// to its SMT sort, and accumulates the uninterpreted functions seen so the
/// document can declare them.
pub struct EncodeCtx {
    sorts: BTreeMap<String, String>,
    ufs: BTreeMap<String, UfSig>,
}

impl EncodeCtx {
    /// Build a context from `(name, smt_sort)` bindings.
    pub fn new(bindings: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            sorts: bindings.into_iter().collect(),
            ufs: BTreeMap::new(),
        }
    }

    /// The SMT sort of a bare name, if known (state field / param / arg).
    fn sort_of_name(&self, name: &str) -> Option<&str> {
        self.sorts.get(name).map(String::as_str)
    }

    /// The uninterpreted-function signatures collected during encoding.
    pub fn ufs(&self) -> &BTreeMap<String, UfSig> {
        &self.ufs
    }
}

/// Value-bearing field-set predicate, the **wide** D4 rule
/// (`is_value_bearing_split`, mirrored from
/// `portrait-sema/src/lib.rs:1054`). A field is value-bearing iff it is
/// `coin`-typed, OR its name is one of `balance`/`amount`/`supply`, OR its name
/// ends in `balance`.
///
/// MUST be the wide rule: the narrow `is_value_bearing` would empty `V` for
/// multi-leg covenants (e.g. `InternalSplit`'s int-typed `pool_*_balance` legs),
/// collapsing the conservation sum to the vacuous `0 = 0` and spuriously PROVING
/// a broken rebalance (the red-team finding). A regression test pins this.
pub fn is_value_bearing_split(name: &str, ty: &Type) -> bool {
    matches!(ty, Type::Coin)
        || name == "balance"
        || name == "amount"
        || name == "supply"
        || name.ends_with("balance")
}

/// `true` if the field is `coin`-sorted (needs the `(>= v 0)` domain axiom on
/// both pre- and post-state, spec §3.3).
pub fn is_coin(field: &Field) -> bool {
    matches!(field.ty, Type::Coin)
}

/// Encode an expression to an SMT-LIB term, tracking the logic it contributes and
/// the result sort expected of this position.
///
/// Returns [`EncodeError`] for nodes that cannot be soundly encoded in the
/// conservation fragment: a non-constant-foldable `Mul` (would need QF_NIA,
/// undecidable — we refuse rather than emit something we cannot stand behind),
/// or a `Bytes` literal in an arithmetic position (out of fragment).
pub fn encode_expr(
    expr: &Expr,
    ctx: &mut EncodeCtx,
    logic: &mut Logic,
    expect: ExpectSort,
) -> Result<String, EncodeError> {
    match expr {
        Expr::Int(n) => Ok(n.to_string()),
        Expr::Bool(b) => Ok(b.to_string()),
        Expr::Var(name) => Ok(sanitize(name)),
        Expr::Unary { op, rhs } => {
            let (inner, op_str) = match op {
                UnOp::Neg => (ExpectSort::Int, "-"),
                UnOp::Not => (ExpectSort::Bool, "not"),
            };
            let r = encode_expr(rhs, ctx, logic, inner)?;
            Ok(format!("({op_str} {r})"))
        }
        Expr::Binary { op, lhs, rhs } => encode_binary(*op, lhs, rhs, ctx, logic),
        // Field / Index / Call are the soundness corners: uninterpreted. Their
        // presence lifts the logic to QF_AUFLIA. Encoded so a property that
        // *depends* on them stays sat (REFUTED/UNKNOWN), never spuriously unsat.
        Expr::Call { name, args } => {
            *logic = logic.join(Logic::QfAuflia);
            let mut arg_sorts = Vec::new();
            let mut encoded = Vec::new();
            for a in args {
                // Argument sort: known if it is a bare name we have a sort for;
                // otherwise default to Int (sound: an opaque UF arg type only
                // affects well-sortedness, never the value-conservation verdict).
                let s = arg_sort(a, ctx);
                arg_sorts.push(s);
                encoded.push(encode_expr(a, ctx, logic, ExpectSort::Int)?);
            }
            let ret = match expect {
                ExpectSort::Bool => "Bool",
                ExpectSort::Int => "Int",
            };
            ctx.ufs.entry(sanitize(name)).or_insert(UfSig {
                args: arg_sorts,
                ret: ret.to_string(),
            });
            if encoded.is_empty() {
                Ok(sanitize(name))
            } else {
                Ok(format!("({} {})", sanitize(name), encoded.join(" ")))
            }
        }
        Expr::Field { base, field } => {
            *logic = logic.join(Logic::QfAuflia);
            let b = encode_expr(base, ctx, logic, ExpectSort::Int)?;
            // Uninterpreted selector `acc_<field>(base)` (spec §2.3).
            let fname = format!("acc_{}", sanitize(field));
            let ret = match expect {
                ExpectSort::Bool => "Bool",
                ExpectSort::Int => "Int",
            };
            ctx.ufs.entry(fname.clone()).or_insert(UfSig {
                args: vec!["Int".to_string()],
                ret: ret.to_string(),
            });
            Ok(format!("({fname} {b})"))
        }
        Expr::Index { base, index } => Err(EncodeError(format!(
            "array index (`{}`) is outside the M1 conservation fragment",
            Expr::Index {
                base: base.clone(),
                index: index.clone()
            }
        ))),
        Expr::Bytes(_) => Err(EncodeError(
            "byte-string literal in an arithmetic/conservation position".to_string(),
        )),
    }
}

/// The SMT sort to attribute to a call argument: its declared sort if it is a
/// known bare name, else `Int` (default; affects only well-sortedness).
fn arg_sort(expr: &Expr, ctx: &EncodeCtx) -> String {
    if let Expr::Var(name) = expr {
        if let Some(s) = ctx.sort_of_name(name) {
            return s.to_string();
        }
    }
    "Int".to_string()
}

fn encode_binary(
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    ctx: &mut EncodeCtx,
    logic: &mut Logic,
) -> Result<String, EncodeError> {
    // `Mul` is the only arithmetic node that can leave the decidable linear
    // fragment. Keep it ONLY when one side folds to an integer constant (linear,
    // exact, stays QF_LIA); otherwise refuse — emitting `(* a b)` would need
    // QF_NIA (undecidable) and we will not stand a `PROVED` on it.
    if op == BinOp::Mul {
        if let Some(k) = const_int(lhs) {
            let r = encode_expr(rhs, ctx, logic, ExpectSort::Int)?;
            return Ok(format!("(* {k} {r})"));
        }
        if let Some(k) = const_int(rhs) {
            let l = encode_expr(lhs, ctx, logic, ExpectSort::Int)?;
            return Ok(format!("(* {l} {k})"));
        }
        return Err(EncodeError(
            "non-constant multiplication (would require undecidable QF_NIA)".to_string(),
        ));
    }
    // Operand sort: comparisons/arithmetic take Int operands; logical take Bool.
    let operand = match op {
        BinOp::And | BinOp::Or => ExpectSort::Bool,
        _ => ExpectSort::Int,
    };
    let l = encode_expr(lhs, ctx, logic, operand)?;
    let r = encode_expr(rhs, ctx, logic, operand)?;
    let smt_op = match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => unreachable!("Mul handled above"),
        BinOp::Eq => "=",
        BinOp::Ne => "distinct",
        BinOp::Ge => ">=",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Lt => "<",
        BinOp::And => "and",
        BinOp::Or => "or",
    };
    Ok(format!("({smt_op} {l} {r})"))
}

/// Fold an expression to a constant `i64` if it is a constant integer (a literal
/// or a negation of a literal). Used to keep `k * e` linear (spec §2.2).
fn const_int(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Int(n) => Some(*n),
        Expr::Unary { op: UnOp::Neg, rhs } => const_int(rhs).map(|n| -n),
        _ => None,
    }
}

/// Sanitize a Portrait identifier into an SMT-LIB symbol. Portrait identifiers
/// are `[A-Za-z_][A-Za-z0-9_]*`, all of which are legal SMT-LIB simple symbols,
/// so this is the identity in practice; kept as a single chokepoint.
pub fn sanitize(name: &str) -> String {
    name.to_string()
}

/// The SMT sort name for a Portrait type (spec §2.1). `Coin` and `Int` are both
/// `Int` (coin carries the `>= 0` domain axiom separately, §3.3); opaque types
/// get a declared uninterpreted sort.
pub fn sort_of(ty: &Type) -> &'static str {
    match ty {
        Type::Int | Type::Coin => "Int",
        Type::Bool => "Bool",
        Type::Bytes32 => "Bytes32",
        Type::PubKey => "PubKey",
        Type::Sig => "Sig",
        Type::Set(_) => "Set_T",
        Type::Named(_) => "Named_T",
        // Map cannot reach a covenant (rejected at parse).
        Type::Map(_, _) => "Named_T",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portrait_syntax::Type;

    fn ctx() -> EncodeCtx {
        EncodeCtx::new([
            ("balance".to_string(), "Int".to_string()),
            ("amount".to_string(), "Int".to_string()),
            ("x".to_string(), "Int".to_string()),
            ("y".to_string(), "Int".to_string()),
            ("auth".to_string(), "Sig".to_string()),
            ("owner".to_string(), "PubKey".to_string()),
        ])
    }

    #[test]
    fn split_predicate_matches_sema_on_worked_example_fields() {
        // MultisigTreasury: V = {balance} (name in VALUE_BEARING_NAMES).
        assert!(is_value_bearing_split("balance", &Type::Int));
        assert!(!is_value_bearing_split("signer_a", &Type::PubKey));
        // InternalSplit: V = {pool_a/b/c_balance} via the *wide* rule (ends in
        // "balance"), even though they are int-typed and not exactly "balance".
        assert!(is_value_bearing_split("pool_a_balance", &Type::Int));
        assert!(is_value_bearing_split("pool_b_balance", &Type::Int));
        assert!(is_value_bearing_split("pool_c_balance", &Type::Int));
        assert!(!is_value_bearing_split("owner", &Type::PubKey));
        // coin-typed is value-bearing regardless of name.
        assert!(is_value_bearing_split("locked", &Type::Coin));
    }

    #[test]
    fn add_sub_stay_qf_lia() {
        let mut logic = Logic::QfLia;
        let e = portrait_syntax::parse_expr("balance - amount").unwrap();
        let t = encode_expr(&e, &mut ctx(), &mut logic, ExpectSort::Int).unwrap();
        assert_eq!(t, "(- balance amount)");
        assert_eq!(logic, Logic::QfLia);
    }

    #[test]
    fn const_mul_stays_linear_and_is_exact() {
        // x * 2 — the case structural D4 cannot reason about (x*2 == x+x).
        let mut logic = Logic::QfLia;
        let e = portrait_syntax::parse_expr("x * 2").unwrap();
        let t = encode_expr(&e, &mut ctx(), &mut logic, ExpectSort::Int).unwrap();
        assert_eq!(t, "(* x 2)");
        assert_eq!(logic, Logic::QfLia);
    }

    #[test]
    fn non_const_mul_is_refused() {
        let mut logic = Logic::QfLia;
        let e = portrait_syntax::parse_expr("x * y").unwrap();
        assert!(encode_expr(&e, &mut ctx(), &mut logic, ExpectSort::Int).is_err());
    }

    #[test]
    fn checksig_call_lifts_to_auflia_and_is_declared_bool() {
        let mut logic = Logic::QfLia;
        let mut c = ctx();
        let e = portrait_syntax::parse_expr("checkSig(auth, owner)").unwrap();
        let t = encode_expr(&e, &mut c, &mut logic, ExpectSort::Bool).unwrap();
        assert_eq!(t, "(checkSig auth owner)");
        assert_eq!(logic, Logic::QfAuflia);
        let sig = c.ufs().get("checkSig").expect("checkSig collected");
        assert_eq!(sig.ret, "Bool");
        assert_eq!(sig.args, vec!["Sig".to_string(), "PubKey".to_string()]);
    }
}
