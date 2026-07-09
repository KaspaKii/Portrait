//! Portrait surface syntax: the AST and the parser entry point.
//! Grammar is pinned in docs/BUILD_SPEC.md §3; `parse` is stubbed for M0.

#[derive(Debug, Clone)]
pub struct Program {
    pub pragma: String,
    pub uses: Vec<String>,
    pub app: App,
}

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub roles: Vec<Role>,
    pub lifecycle: Vec<Edge>,
    pub flow: Option<Flow>,
    pub invariants: Vec<Invariant>,
}

#[derive(Debug, Clone)]
pub struct Role {
    pub name: String,
    pub component: Option<String>,
    pub params: Vec<Param>,
    pub state: Vec<Field>,
    pub entrypoints: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Bool,
    PubKey,
    Sig,
    Bytes32,
    Coin,
    Set(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Named(String),
}

#[derive(Debug, Clone)]
pub enum CovenantMode {
    Transition,
    Verification,
    NonCovenant,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub mode: CovenantMode,
    pub args: Vec<Param>,
    pub returns: Option<Type>,
    pub requires: Vec<String>,
    pub body: Vec<Stmt>,
}

/// A typed expression statement body. `Require`/`Return` now carry a parsed
/// `Expr`/`ReturnExpr` tree (Phase B1); `Raw` remains as the parse-failure
/// fallback so any construct the precedence parser cannot yet handle is still
/// carried through verbatim (an untyped hole the checker/red-team must see).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Require(Expr),
    Return(ReturnExpr),
    Raw(String),
}

/// Binary operators, low→high precedence handled by the Pratt parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
    And,
    Or,
}

impl BinOp {
    /// The silverscript surface spelling of this operator.
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Ge => ">=",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Lt => "<",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

impl UnOp {
    pub fn as_str(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
        }
    }
}

/// The typed surface expression AST. `to_silverscript()`/`Display` reproduce a
/// canonical infix rendering so downstream string consumers (portrait-emit's
/// `lower_return_expr`, portrait-atelier's guest lowering) keep working
/// transitionally during Phase B.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    Bytes(Vec<u8>),
    Var(String),
    Field {
        base: Box<Expr>,
        field: String,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Unary {
        op: UnOp,
        rhs: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

impl Expr {
    /// Render this expression back to silverscript surface syntax. The output is
    /// re-parseable by the downstream string-substitution code (it only relies
    /// on whole-identifier tokens and balanced delimiters), so canonical
    /// whitespace here is intentional and safe.
    pub fn to_silverscript(&self) -> String {
        match self {
            Expr::Int(n) => n.to_string(),
            Expr::Bool(b) => b.to_string(),
            Expr::Bytes(bytes) => {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                format!("0x{hex}")
            }
            Expr::Var(name) => name.clone(),
            Expr::Field { base, field } => format!("{}.{}", base.to_silverscript(), field),
            Expr::Index { base, index } => {
                format!("{}[{}]", base.to_silverscript(), index.to_silverscript())
            }
            Expr::Unary { op, rhs } => format!("{}{}", op.as_str(), rhs.to_silverscript()),
            Expr::Binary { op, lhs, rhs } => format!(
                "{} {} {}",
                lhs.to_silverscript(),
                op.as_str(),
                rhs.to_silverscript()
            ),
            Expr::Call { name, args } => {
                let rendered: Vec<String> = args.iter().map(Expr::to_silverscript).collect();
                format!("{}({})", name, rendered.join(", "))
            }
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_silverscript())
    }
}

/// A `return` expression. Either a scalar (`return value + delta;`) or a state
/// object literal (`return Name { f1: v1, f2: v2, ... };`).
#[derive(Debug, Clone, PartialEq)]
pub enum ReturnExpr {
    /// Scalar return: a single expression.
    Scalar(Expr),
    /// Object literal: an optional head name plus ordered `(field, value)` pairs.
    Object {
        name: Option<String>,
        fields: Vec<(String, Expr)>,
    },
}

impl ReturnExpr {
    /// Render back to silverscript surface syntax, re-parseable by
    /// portrait-emit's `parse_object_literal` (head + `{ f: v, ... }`).
    pub fn to_silverscript(&self) -> String {
        match self {
            ReturnExpr::Scalar(expr) => expr.to_silverscript(),
            ReturnExpr::Object { name, fields } => {
                let head = name.clone().unwrap_or_default();
                let rendered: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_silverscript()))
                    .collect();
                if head.is_empty() {
                    format!("{{ {} }}", rendered.join(", "))
                } else {
                    format!("{} {{ {} }}", head, rendered.join(", "))
                }
            }
        }
    }
}

impl std::fmt::Display for ReturnExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_silverscript())
    }
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub via_role: String,
    pub via_entry: String,
    pub terminal: bool,
}

#[derive(Debug, Clone)]
pub struct Flow {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub enum Step {
    Move { role: String, entry: String },
    Choose(Vec<Flow>),
    Par(Vec<Flow>),
    Repeat(u32, Box<Flow>),
}

#[derive(Debug, Clone)]
pub enum Invariant {
    ValueConserved,
    NoUndeclaredState,
    Custom(String),
}

/// One entry in the Solidity-subset *rejection set*: a construct that cannot be
/// projected onto a Kaspa UTXO covenant and is therefore the contract of the
/// vProgs (Tier-3) layer. The `lead` is the leading identifier the parser sees
/// at the head of an out-of-subset statement (or, for method-call forms, an
/// identifier that appears as a `.<lead>(` call). `reason` names *why* it does
/// not project; `subset_ref` cites the documented item in SOLIDITY-SUBSET-V0 §3.
///
/// This table is the single source of truth that the fail-loud check in
/// `parse_block` reads, so the prose doc (SOLIDITY-SUBSET-V0 §3) and the code
/// check cannot drift. Before this existed, an out-of-subset statement silently
/// degraded to `Stmt::Raw`; now each blacklisted construct is rejected with a
/// diagnostic that names it and routes it to the vProgs layer.
#[derive(Debug, Clone, Copy)]
pub struct RejectedConstruct {
    /// Leading identifier (statement head) or method name (`.<lead>(`) that flags
    /// this construct.
    pub lead: &'static str,
    /// Whether `lead` is matched as a statement head (`false`) or as a
    /// `.<lead>(` method-call anywhere in the statement (`true`).
    pub as_method_call: bool,
    /// Human-readable reason the construct does not project onto a covenant.
    pub reason: &'static str,
    /// The documented item in `docs/SOLIDITY-SUBSET-V0.md §3`.
    pub subset_ref: &'static str,
}

/// The explicit, fail-loud rejection set. Mirrors `docs/SOLIDITY-SUBSET-V0.md §3`.
/// Each construct here is *deferred to the vProgs layer*: the rejection set is
/// not a failure, it is the Tier-3 contract.
pub const REJECTION_SET: &[RejectedConstruct] = &[
    RejectedConstruct {
        lead: "for",
        as_method_call: false,
        reason: "unbounded loops cannot be expressed in a covenant (script has no \
                 general iteration)",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 9 (dynamic/unbounded forms)",
    },
    RejectedConstruct {
        lead: "while",
        as_method_call: false,
        reason: "unbounded loops cannot be expressed in a covenant (script has no \
                 general iteration)",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 9 (dynamic/unbounded forms)",
    },
    RejectedConstruct {
        lead: "emit",
        as_method_call: false,
        reason: "event logs have no native equivalent on Kaspa; reconstruct from the \
                 UTXO graph off-chain",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 7 (event logs)",
    },
    RejectedConstruct {
        lead: "mapping",
        as_method_call: false,
        reason: "shared mutable mappings require an account model; the UTXO model has \
                 no global mutable ledger",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 1 (shared mutable mappings)",
    },
    RejectedConstruct {
        lead: "approve",
        as_method_call: false,
        reason: "the approve/transferFrom allowance pattern needs a global allowance \
                 ledger",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 2 (approve/transferFrom)",
    },
    RejectedConstruct {
        lead: "transferFrom",
        as_method_call: false,
        reason: "the approve/transferFrom allowance pattern needs a global allowance \
                 ledger",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 2 (approve/transferFrom)",
    },
    RejectedConstruct {
        lead: "delegatecall",
        as_method_call: true,
        reason: "synchronous cross-contract calls require an account model or vProg \
                 composition",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 3 (synchronous cross-contract calls)",
    },
    RejectedConstruct {
        lead: "call",
        as_method_call: true,
        reason: "synchronous cross-contract calls require an account model or vProg \
                 composition",
        subset_ref: "SOLIDITY-SUBSET-V0 §3 item 3 (synchronous cross-contract calls)",
    },
];

/// If the statement that starts at the parser's current position leads with a
/// blacklisted construct, return its rejection-set entry. Statement-head matches
/// look only at the leading identifier; method-call matches scan forward to the
/// terminating `;` for a `.<lead>(` shape. This is intentionally conservative:
/// it only fires on the documented rejection set, leaving genuinely-unrecognised
/// (but not blacklisted) forms to the `Stmt::Raw` path.
fn rejected_construct_at(parser: &Parser) -> Option<RejectedConstruct> {
    let head = match parser.peek().map(|t| &t.kind) {
        Some(TokenKind::Ident(s)) => Some(s.as_str()),
        _ => None,
    };
    // 1. Statement-head match (e.g. `for`, `while`, `emit`, `mapping`, `approve`).
    if let Some(h) = head {
        if let Some(rc) = REJECTION_SET
            .iter()
            .find(|rc| !rc.as_method_call && rc.lead == h)
        {
            return Some(*rc);
        }
    }
    // 2. Method-call match (e.g. `target.call(...)`, `lib.delegatecall(...)`):
    //    scan the statement's tokens up to the terminating `;` for the shape
    //    `. <lead> (`.
    let mut i = parser.pos;
    while let Some(tok) = parser.tokens.get(i) {
        if matches!(tok.kind, TokenKind::Symbol(';')) {
            break;
        }
        if matches!(tok.kind, TokenKind::Symbol('.')) {
            if let (Some(TokenKind::Ident(name)), Some(TokenKind::Symbol('('))) = (
                parser.tokens.get(i + 1).map(|t| &t.kind),
                parser.tokens.get(i + 2).map(|t| &t.kind),
            ) {
                if let Some(rc) = REJECTION_SET
                    .iter()
                    .find(|rc| rc.as_method_call && rc.lead == name.as_str())
                {
                    return Some(*rc);
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse Portrait source into a Program. M0: stubbed (grammar in BUILD_SPEC §3.2).
pub fn parse(src: &str) -> Result<Program, String> {
    Parser::new(src).parse_program()
}

/// Parse a bare `return` expression (without the `return` keyword or `;`) into a
/// [`ReturnExpr`]. Accepts a scalar expression or a `Name { f: v, ... }` object
/// literal. Used by downstream crates' fixtures to build typed bodies from the
/// silverscript surface form during the Phase B transition.
pub fn parse_return_expr(src: &str) -> Result<ReturnExpr, String> {
    let with_terminator = format!("{src};");
    let mut parser = Parser::new(&with_terminator);
    let ret = parser.try_parse_return()?;
    if parser.peek().is_some() {
        return Err("trailing tokens after return expression".to_string());
    }
    Ok(ret)
}

/// Parse a bare scalar expression (no `;`) into an [`Expr`]. Used by downstream
/// fixtures during the Phase B transition.
pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let mut parser = Parser::new(src);
    let expr = parser.parse_expr(0)?;
    if parser.peek().is_some() {
        return Err("trailing tokens after expression".to_string());
    }
    Ok(expr)
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Ident(String),
    Number(String),
    Symbol(char),
    Arrow,
    Le,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            tokens: tokenize(src),
            pos: 0,
        }
    }

    fn parse_program(&mut self) -> Result<Program, String> {
        self.expect_keyword("pragma")?;
        let pragma = self.parse_pragma_payload()?;

        let mut uses = Vec::new();
        while self.peek_keyword("use") {
            self.bump();
            let start = self
                .peek()
                .ok_or_else(|| self.error("expected use path"))?
                .start;
            self.consume_until(';')?;
            let end = self.prev_end();
            uses.push(self.slice(start, end));
            self.expect_symbol(';')?;
        }

        let app = self.parse_app()?;
        if self.peek().is_some() {
            return Err(self.error("unexpected trailing tokens after app"));
        }

        Ok(Program { pragma, uses, app })
    }

    fn parse_pragma_payload(&mut self) -> Result<String, String> {
        let start = self
            .peek()
            .ok_or_else(|| self.error("expected pragma payload"))?
            .start;
        self.consume_until(';')?;
        let end = self.prev_end();
        let payload = self.slice(start, end);
        self.expect_symbol(';')?;
        Ok(payload)
    }

    fn parse_app(&mut self) -> Result<App, String> {
        self.expect_keyword("app")?;
        let name = self.expect_ident()?;
        self.expect_symbol('{')?;

        let mut roles = Vec::new();
        let mut lifecycle = Vec::new();
        let mut flow = None;
        let mut invariants = Vec::new();

        while !self.peek_symbol('}') {
            if self.peek_keyword("role") {
                roles.push(self.parse_role()?);
            } else if self.peek_keyword("lifecycle") {
                lifecycle = self.parse_lifecycle()?;
            } else if self.peek_keyword("flow") {
                flow = Some(self.parse_flow()?);
            } else if self.peek_keyword("invariant") {
                invariants.push(self.parse_invariant()?);
            } else if self.peek_keyword("contract") {
                // App-composition grammar (`app X { contract v = Type { ... } }`)
                // is NOT a covenant source. The covenant form this compiler
                // engraves uses role/lifecycle/flow/invariant. Emit a clear,
                // actionable diagnostic instead of a cryptic byte-offset error.
                return Err(self.error(
                    "app-composition sources (`contract <name> = <Type> { ... }`) are not \
                     covenant sources; the covenant form uses role/lifecycle/flow/invariant \
                     — see TimeVault.portrait for the canonical covenant grammar",
                ));
            } else {
                return Err(self.error("expected role, lifecycle, flow, or invariant"));
            }
        }

        self.expect_symbol('}')?;
        Ok(App {
            name,
            roles,
            lifecycle,
            flow,
            invariants,
        })
    }

    fn parse_role(&mut self) -> Result<Role, String> {
        self.expect_keyword("role")?;
        let name = self.expect_ident()?;
        let component = if self.peek_symbol('=') {
            self.bump();
            Some(self.expect_ident()?)
        } else {
            None
        };
        self.expect_symbol('{')?;

        let mut params = Vec::new();
        let mut state = Vec::new();
        let mut entrypoints = Vec::new();

        while !self.peek_symbol('}') {
            if self.peek_keyword("param") {
                params.push(self.parse_param()?);
            } else if self.peek_keyword("state") {
                state = self.parse_state_block()?;
            } else if self.peek_symbol('#') {
                let mode = self.parse_covenant_attr()?;
                entrypoints.push(self.parse_entrypoint(mode)?);
            } else if self.peek_keyword("entrypoint") {
                entrypoints.push(self.parse_entrypoint(CovenantMode::NonCovenant)?);
            } else {
                return Err(self.error("expected param, state, or entrypoint"));
            }
        }

        self.expect_symbol('}')?;
        Ok(Role {
            name,
            component,
            params,
            state,
            entrypoints,
        })
    }

    fn parse_param(&mut self) -> Result<Param, String> {
        self.expect_keyword("param")?;
        let ty = self.parse_type()?;
        let name = self.expect_ident()?;
        if self.peek_symbol('=') {
            self.bump();
            self.consume_until(';')?;
            return Err(self.error("defaulted params are not supported in M0"));
        }
        self.expect_symbol(';')?;
        Ok(Param {
            name,
            ty,
            default: None,
        })
    }

    fn parse_state_block(&mut self) -> Result<Vec<Field>, String> {
        self.expect_keyword("state")?;
        self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.peek_symbol('}') {
            let ty_pos = self.peek().map(|t| t.start).unwrap_or(0);
            let ty = self.parse_type()?;
            // Fail-loud rejection set: a `map<K, V>` state field is a shared
            // mutable mapping, which cannot live in a single-UTXO covenant state.
            // Name it and route to vProgs rather than silently lowering it.
            if matches!(ty, Type::Map(_, _)) {
                return Err(self.error_at(
                    ty_pos,
                    "unsupported construct `map<K, V>` state field: shared mutable mappings \
                     require an account model; the UTXO model has no global mutable ledger \
                     (deferred to the vProgs layer; see SOLIDITY-SUBSET-V0 §3 item 1)",
                ));
            }
            let name = self.expect_ident()?;
            self.expect_symbol(';')?;
            fields.push(Field { name, ty });
        }
        self.expect_symbol('}')?;
        Ok(fields)
    }

    fn parse_covenant_attr(&mut self) -> Result<CovenantMode, String> {
        self.expect_symbol('#')?;
        self.expect_symbol('[')?;
        self.expect_keyword("covenant")?;
        if self.peek_symbol('.') {
            self.bump();
            self.expect_keyword("singleton")?;
        }
        self.expect_symbol('(')?;
        self.expect_keyword("mode")?;
        self.expect_symbol('=')?;
        let mode = match self.expect_ident()?.as_str() {
            "transition" => CovenantMode::Transition,
            "verification" => CovenantMode::Verification,
            other => return Err(self.error(&format!("unknown covenant mode {other}"))),
        };
        self.expect_symbol(')')?;
        self.expect_symbol(']')?;
        Ok(mode)
    }

    fn parse_entrypoint(&mut self, mode: CovenantMode) -> Result<Entry, String> {
        self.expect_keyword("entrypoint")?;
        self.expect_keyword("function")?;
        let name = self.expect_ident()?;
        self.expect_symbol('(')?;
        let args = if self.peek_symbol(')') {
            Vec::new()
        } else {
            self.parse_args()?
        };
        self.expect_symbol(')')?;

        let returns = if self.peek_symbol(':') {
            self.bump();
            self.expect_symbol('(')?;
            let ty = self.parse_type()?;
            if self.peek_ident() {
                self.bump();
            }
            while self.peek_symbol(',') {
                self.bump();
                self.consume_until_symbol(')')?;
                if self.peek_ident() {
                    self.bump();
                }
            }
            self.expect_symbol(')')?;
            Some(ty)
        } else {
            None
        };

        let body = self.parse_block(&mode)?;
        let requires = body
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Require(expr) => Some(expr.to_silverscript()),
                _ => None,
            })
            .collect();

        Ok(Entry {
            name,
            mode,
            args,
            returns,
            requires,
            body,
        })
    }

    fn parse_args(&mut self) -> Result<Vec<Param>, String> {
        let mut args = Vec::new();
        loop {
            let ty = self.parse_type()?;
            let name = self.expect_ident()?;
            args.push(Param {
                name,
                ty,
                default: None,
            });
            if self.peek_symbol(',') {
                self.bump();
                continue;
            }
            break;
        }
        Ok(args)
    }

    /// Parse an entrypoint body. `mode` is the enclosing entrypoint's covenant
    /// mode and drives the *allocation-aware* handling of the REJECTION_SET: a
    /// blacklisted construct in a COVENANT entrypoint (`Transition`/`Verification`)
    /// is loud-rejected (byte-identical to before); the SAME construct in a
    /// NonCovenant (vProg) entrypoint is ACCEPTED as a recorded `Stmt::Raw` hole,
    /// because vProgs (Tier-3) are exactly where loops/mappings/cross-calls belong.
    /// State-field mappings stay unconditionally rejected (see `parse_state_block`):
    /// state is shared covenant UTXO state, mode-independent.
    fn parse_block(&mut self, mode: &CovenantMode) -> Result<Vec<Stmt>, String> {
        self.expect_symbol('{')?;
        let mut body = Vec::new();
        while !self.peek_symbol('}') {
            if self.peek_keyword("return") {
                self.bump();
                body.push(self.parse_return_stmt(mode)?);
            } else if self.peek_keyword("require") || self.peek_keyword("requires") {
                self.bump();
                body.push(self.parse_require_stmt(mode)?);
            } else if let Some(rc) = rejected_construct_at(self) {
                // Allocation-aware rejection set. In a vProg entrypoint, the
                // construct is legitimate — accept it as a recorded Raw hole that
                // survives to sema (which already tolerates Raw in NonCovenant
                // entrypoints). In a covenant entrypoint, keep the EXACT fail-loud
                // error: the rejection set IS the Tier-3 contract for covenants.
                if matches!(mode, CovenantMode::NonCovenant) {
                    body.push(self.consume_rejected_as_raw()?);
                    continue;
                }
                let pos = self.peek().map(|t| t.start).unwrap_or(0);
                return Err(self.error_at(
                    pos,
                    &format!(
                        "unsupported construct `{}`: {} (deferred to the vProgs layer; see {})",
                        rc.lead, rc.reason, rc.subset_ref
                    ),
                ));
            } else {
                // Genuinely-unrecognised (but not blacklisted) form: carry
                // verbatim as Raw for the checker/red-team to see.
                body.push(self.parse_raw_stmt("statement")?);
                continue;
            }
        }
        self.expect_symbol('}')?;
        Ok(body)
    }

    /// Parse `return <expr>;` or `return Name { f: v, ... };`. On any parse
    /// failure, rewind to the expression's position and (1) consult the
    /// single-source-of-truth `REJECTION_SET` — a blacklisted construct embedded
    /// in the return (e.g. `return target.call(x);`) fail-loud rejects, naming the
    /// construct, identically to the standalone path; otherwise (2) capture the
    /// whole statement as `Stmt::Raw` (logged), so emission still has a verbatim
    /// string to lower.
    fn parse_return_stmt(&mut self, mode: &CovenantMode) -> Result<Stmt, String> {
        let checkpoint = self.pos;
        match self.try_parse_return() {
            Ok(ret) => Ok(Stmt::Return(ret)),
            Err(_) => {
                self.pos = checkpoint;
                self.reject_or_raw("return", mode)
            }
        }
    }

    fn try_parse_return(&mut self) -> Result<ReturnExpr, String> {
        // Object literal: `Name? { field: expr, ... }`.
        if self.peek_symbol('{') {
            let ret = self.parse_object_literal(None)?;
            self.expect_symbol(';')?;
            return Ok(ret);
        }
        if self.peek_ident() && self.peek_symbol_at(1, '{') {
            let name = self.expect_ident()?;
            let ret = self.parse_object_literal(Some(name))?;
            self.expect_symbol(';')?;
            return Ok(ret);
        }
        // Scalar expression.
        let expr = self.parse_expr(0)?;
        self.expect_symbol(';')?;
        Ok(ReturnExpr::Scalar(expr))
    }

    fn parse_object_literal(&mut self, name: Option<String>) -> Result<ReturnExpr, String> {
        self.expect_symbol('{')?;
        let mut fields = Vec::new();
        while !self.peek_symbol('}') {
            let field = self.expect_ident()?;
            self.expect_symbol(':')?;
            let value = self.parse_expr(0)?;
            fields.push((field, value));
            if self.peek_symbol(',') {
                self.bump();
            } else {
                break;
            }
        }
        self.expect_symbol('}')?;
        if fields.is_empty() {
            return Err(self.error("empty object literal in return"));
        }
        Ok(ReturnExpr::Object { name, fields })
    }

    /// Parse `require(<expr>);` / `require <expr>;`. On failure, rewind and
    /// consult the `REJECTION_SET` first (so a blacklisted construct embedded in a
    /// require — e.g. `require strategy.call(amount);` — fail-loud rejects naming
    /// the construct, identically to the standalone path) before degrading to Raw.
    fn parse_require_stmt(&mut self, mode: &CovenantMode) -> Result<Stmt, String> {
        let checkpoint = self.pos;
        match self.try_parse_require() {
            Ok(expr) => Ok(Stmt::Require(expr)),
            Err(_) => {
                self.pos = checkpoint;
                self.reject_or_raw("require", mode)
            }
        }
    }

    fn try_parse_require(&mut self) -> Result<Expr, String> {
        // Accept both `require(expr);` and `require expr;`.
        let parenthesised = self.peek_symbol('(');
        if parenthesised {
            self.bump();
        }
        let expr = self.parse_expr(0)?;
        if parenthesised {
            self.expect_symbol(')')?;
        }
        self.expect_symbol(';')?;
        Ok(expr)
    }

    /// Build a `Stmt::Raw` for an unknown statement form, advancing past `;`.
    /// `kind` is reserved for callers that want to label the construct.
    fn parse_raw_stmt(&mut self, _kind: &str) -> Result<Stmt, String> {
        let start = self
            .peek()
            .ok_or_else(|| self.error("expected statement"))?
            .start;
        self.consume_until(';')?;
        let end = self.prev_end();
        let text = self.slice(start, end);
        self.expect_symbol(';')?;
        Ok(Stmt::Raw(text))
    }

    /// Consume a REJECTION_SET construct that is legitimate in a vProg body as a
    /// recorded `Stmt::Raw` hole. Unlike `parse_raw_stmt`/`reject_or_raw`, this is
    /// DEPTH-AWARE: a `for (i = 0; i < n; ...) { ... }` has semicolons *inside* the
    /// parens and a nested brace block, so a naive "skip to `;`" would stop early
    /// and corrupt the parse. Track `(`/`)` and `{`/`}` depth; the statement ends
    /// at the first `;` seen at depth 0, OR at the `}` that closes a depth-0 brace
    /// block (block-form statement, no trailing `;`).
    fn consume_rejected_as_raw(&mut self) -> Result<Stmt, String> {
        let start = self
            .peek()
            .ok_or_else(|| self.error("expected statement"))?
            .start;
        let mut paren_depth: i32 = 0;
        let mut brace_depth: i32 = 0;
        let mut saw_block = false;
        while let Some(tok) = self.peek() {
            match &tok.kind {
                TokenKind::Symbol('(') => paren_depth += 1,
                TokenKind::Symbol(')') => paren_depth -= 1,
                TokenKind::Symbol('{') => {
                    brace_depth += 1;
                    saw_block = true;
                }
                TokenKind::Symbol('}') => {
                    // A depth-0 `}` closing this block ends a block-form statement
                    // (no trailing `;`). But the entrypoint's own closing `}` must
                    // not be consumed here: that only happens at brace_depth 0 with
                    // no block opened, which we guard with `saw_block`.
                    if brace_depth == 0 {
                        break;
                    }
                    brace_depth -= 1;
                    if brace_depth == 0 && paren_depth == 0 {
                        // Closed the construct's block. Consume the `}` and stop;
                        // an optional trailing `;` is swallowed below.
                        self.pos += 1;
                        let end = self.prev_end();
                        if self.peek_symbol(';') {
                            self.bump();
                        }
                        let text = self.slice(start, end);
                        return Ok(Stmt::Raw(text));
                    }
                }
                TokenKind::Symbol(';') if paren_depth == 0 && brace_depth == 0 => {
                    // Statement-form terminator at top level.
                    let end = self.prev_end();
                    self.bump(); // consume the ';'
                    let text = self.slice(start, end);
                    return Ok(Stmt::Raw(text));
                }
                _ => {}
            }
            self.pos += 1;
        }
        // Ran out of tokens (or hit the entrypoint's closing `}`): capture what we
        // have. `saw_block` distinguishes a block-form that the loop above already
        // returned from; reaching here means an unterminated statement — record it.
        let _ = saw_block;
        let end = self.prev_end();
        let text = self.slice(start, end);
        Ok(Stmt::Raw(text))
    }

    /// Rewind has already happened (pos is at the start of the expression that
    /// failed to parse). FIRST consult the single-source-of-truth `REJECTION_SET`
    /// (via `rejected_construct_at`, which scans this statement to `;` for a
    /// blacklisted `.<lead>(` method call or statement head): if a blacklisted
    /// construct is embedded here, fail-loud reject naming it and routing it to
    /// the vProgs layer — the *same* diagnostic the standalone `parse_block` path
    /// emits, so embedding inside require/return can no longer bypass it. Only a
    /// genuinely-unrecognised (non-blacklisted) form degrades to `Stmt::Raw`.
    fn reject_or_raw(&mut self, kind: &str, mode: &CovenantMode) -> Result<Stmt, String> {
        if let Some(rc) = rejected_construct_at(self) {
            // Allocation-aware: a blacklisted construct embedded in a
            // require/return of a vProg (NonCovenant) entrypoint is legitimate —
            // degrade it to a recorded Raw hole (depth-aware skip) rather than
            // loud-rejecting. In a covenant entrypoint the fail-loud error is
            // unchanged (the embedded-vector regression suite pins this).
            if matches!(mode, CovenantMode::NonCovenant) {
                return self.consume_rejected_as_raw();
            }
            let pos = self.peek().map(|t| t.start).unwrap_or(0);
            return Err(self.error_at(
                pos,
                &format!(
                    "unsupported construct `{}`: {} (deferred to the vProgs layer; see {})",
                    rc.lead, rc.reason, rc.subset_ref
                ),
            ));
        }
        let start = self.peek().map(|t| t.start).unwrap_or(0);
        // Skip to the terminating `;` (best-effort; if absent, consume to end).
        while let Some(tok) = self.peek() {
            if matches!(tok.kind, TokenKind::Symbol(';')) {
                break;
            }
            self.pos += 1;
        }
        let end = self.prev_end();
        let text = self.slice(start, end);
        // Consume the `;` if present.
        if self.peek_symbol(';') {
            self.bump();
        }
        eprintln!("portrait-syntax: {kind} body fell back to Stmt::Raw: {text:?}");
        Ok(Stmt::Raw(text))
    }

    // ── Pratt / precedence-climbing expression parser ──────────────────────
    //
    // Precedence (low → high):
    //   ||  <  &&  <  comparison  <  +/-  <  *  <  unary -/!  <  postfix .field
    //   /[idx]/(args)  <  primary.

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, String> {
        let mut lhs = self.parse_unary()?;
        // Stop when the next token is not a binary operator, or binds less
        // tightly than the caller's minimum binding power.
        while let Some((op, _lbp, rbp)) = self.peek_binop().filter(|&(_, lbp, _)| lbp >= min_bp) {
            self.bump_binop();
            let rhs = self.parse_expr(rbp)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// Binding powers for binary operators. Returns `(op, left_bp, right_bp)`;
    /// left-associative so `right_bp = left_bp + 1`.
    fn peek_binop(&self) -> Option<(BinOp, u8, u8)> {
        let op = match self.peek().map(|t| &t.kind)? {
            TokenKind::OrOr => BinOp::Or,
            TokenKind::AndAnd => BinOp::And,
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::Ne => BinOp::Ne,
            TokenKind::Ge => BinOp::Ge,
            TokenKind::Le => BinOp::Le,
            TokenKind::Symbol('>') => BinOp::Gt,
            TokenKind::Symbol('<') => BinOp::Lt,
            TokenKind::Symbol('+') => BinOp::Add,
            TokenKind::Symbol('-') => BinOp::Sub,
            TokenKind::Symbol('*') => BinOp::Mul,
            _ => return None,
        };
        let lbp = match op {
            BinOp::Or => 1,
            BinOp::And => 3,
            BinOp::Eq | BinOp::Ne | BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => 5,
            BinOp::Add | BinOp::Sub => 7,
            BinOp::Mul => 9,
        };
        Some((op, lbp, lbp + 1))
    }

    fn bump_binop(&mut self) {
        self.bump();
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.peek_symbol('-') {
            self.bump();
            let rhs = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                rhs: Box::new(rhs),
            });
        }
        if self.peek_symbol('!') {
            self.bump();
            let rhs = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::Not,
                rhs: Box::new(rhs),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.peek_symbol('.') {
                self.bump();
                let field = self.expect_ident()?;
                expr = Expr::Field {
                    base: Box::new(expr),
                    field,
                };
            } else if self.peek_symbol('[') {
                self.bump();
                let index = self.parse_expr(0)?;
                self.expect_symbol(']')?;
                expr = Expr::Index {
                    base: Box::new(expr),
                    index: Box::new(index),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        // Parenthesised sub-expression.
        if self.peek_symbol('(') {
            self.bump();
            let expr = self.parse_expr(0)?;
            self.expect_symbol(')')?;
            return Ok(expr);
        }
        match self.bump() {
            Some(Token {
                kind: TokenKind::Number(n),
                start,
                ..
            }) => {
                let value = n
                    .parse::<i64>()
                    .map_err(|_| self.error_at(start, "integer literal out of range"))?;
                Ok(Expr::Int(value))
            }
            Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) => {
                match name.as_str() {
                    "true" => return Ok(Expr::Bool(true)),
                    "false" => return Ok(Expr::Bool(false)),
                    _ => {}
                }
                // Call: identifier immediately followed by `(`.
                if self.peek_symbol('(') {
                    self.bump();
                    let mut args = Vec::new();
                    if !self.peek_symbol(')') {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek_symbol(',') {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_symbol(')')?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(tok) => Err(self.error_at(tok.start, "expected an expression")),
            None => Err(self.error("unexpected end of input in expression")),
        }
    }

    fn peek_symbol_at(&self, offset: usize, symbol: char) -> bool {
        matches!(
            self.tokens.get(self.pos + offset).map(|t| &t.kind),
            Some(TokenKind::Symbol(c)) if *c == symbol
        )
    }

    fn parse_lifecycle(&mut self) -> Result<Vec<Edge>, String> {
        self.expect_keyword("lifecycle")?;
        self.expect_symbol('{')?;
        let mut edges = Vec::new();
        while !self.peek_symbol('}') {
            let from = self.expect_ident()?;
            self.expect_arrow()?;
            let to = self.expect_ident()?;
            self.expect_keyword("via")?;
            let via_role = self.expect_ident()?;
            self.expect_symbol('.')?;
            let via_entry = self.expect_ident()?;
            let terminal = if self.peek_keyword("terminal") {
                self.bump();
                true
            } else {
                false
            };
            self.expect_symbol(';')?;
            edges.push(Edge {
                from,
                to,
                via_role,
                via_entry,
                terminal,
            });
        }
        self.expect_symbol('}')?;
        Ok(edges)
    }

    fn parse_flow(&mut self) -> Result<Flow, String> {
        self.expect_keyword("flow")?;
        self.expect_symbol('{')?;
        let mut steps = Vec::new();
        while !self.peek_symbol('}') {
            steps.push(self.parse_flow_step()?);
            if self.peek_symbol(';') {
                self.bump();
            }
        }
        self.expect_symbol('}')?;
        Ok(Flow { steps })
    }

    /// Parse one step of a `flow {}` block. The four step shapes are
    /// distinguished by their head token:
    ///
    /// - `choose { branch { .. } branch { .. } .. }` → [`Step::Choose`]: an
    ///   internal choice; each `branch { .. }` is a nested step sequence.
    /// - `par { thread { .. } thread { .. } .. }` → [`Step::Par`]: parallel
    ///   composition; each `thread { .. }` is a nested step sequence.
    /// - `repeat <N> { .. }` → [`Step::Repeat`]: a bounded loop repeating the
    ///   `{ .. }` step body `N` times (`N` a positive integer literal; `0` is a
    ///   parse error — a loop that never runs is not authorable as a loop).
    /// - `<role>.<entry>` → [`Step::Move`]: the existing handoff form.
    ///
    /// The control keywords (`choose`/`par`/`repeat`) only have this meaning at
    /// the head of a step; a Move always begins `<ident>.`, so the forms do not
    /// collide.
    fn parse_flow_step(&mut self) -> Result<Step, String> {
        if self.peek_keyword("choose") {
            return self.parse_choose_step();
        }
        if self.peek_keyword("par") {
            return self.parse_par_step();
        }
        if self.peek_keyword("repeat") {
            return self.parse_repeat_step();
        }
        let role = self.expect_ident()?;
        self.expect_symbol('.')?;
        let entry = self.expect_ident()?;
        Ok(Step::Move { role, entry })
    }

    /// `choose { branch { <steps> } branch { <steps> } .. }`.
    fn parse_choose_step(&mut self) -> Result<Step, String> {
        self.expect_keyword("choose")?;
        let branches = self.parse_labelled_flows("branch")?;
        Ok(Step::Choose(branches))
    }

    /// `par { thread { <steps> } thread { <steps> } .. }`.
    fn parse_par_step(&mut self) -> Result<Step, String> {
        self.expect_keyword("par")?;
        let threads = self.parse_labelled_flows("thread")?;
        Ok(Step::Par(threads))
    }

    /// `repeat <N> { <steps> }`.
    fn parse_repeat_step(&mut self) -> Result<Step, String> {
        self.expect_keyword("repeat")?;
        let count = self.expect_count()?;
        let body = self.parse_flow_body()?;
        Ok(Step::Repeat(count, Box::new(body)))
    }

    /// Parse `{ <label> { <steps> } <label> { <steps> } .. }`: a brace-delimited
    /// list of one-or-more `<label> { .. }` nested step sequences. Used for the
    /// branches of a `choose` and the threads of a `par`. At least one labelled
    /// sub-flow is required; an empty list (`choose {}`) is a parse error so the
    /// degenerate shape never reaches the lift silently.
    fn parse_labelled_flows(&mut self, label: &str) -> Result<Vec<Flow>, String> {
        self.expect_symbol('{')?;
        let mut flows = Vec::new();
        while !self.peek_symbol('}') {
            self.expect_keyword(label)?;
            flows.push(self.parse_flow_body()?);
        }
        if flows.is_empty() {
            return Err(self.error(&format!("expected at least one `{label} {{ .. }}`")));
        }
        self.expect_symbol('}')?;
        Ok(flows)
    }

    /// Parse a `{ <steps> }` step sequence: the body shared by `branch`,
    /// `thread`, and `repeat`. Reuses the same step grammar as a top-level
    /// `flow {}`, so nesting is uniform.
    fn parse_flow_body(&mut self) -> Result<Flow, String> {
        self.expect_symbol('{')?;
        let mut steps = Vec::new();
        while !self.peek_symbol('}') {
            steps.push(self.parse_flow_step()?);
            if self.peek_symbol(';') {
                self.bump();
            }
        }
        self.expect_symbol('}')?;
        Ok(Flow { steps })
    }

    /// Parse a positive integer literal repeat count into the `u32` the
    /// [`Step::Repeat`] AST carries. `0` is rejected: `repeat 0` means "run the
    /// body zero times", but the M2 lift models any `repeat` as an *unbounded*
    /// loop, so a `repeat 0` would silently lift to the opposite meaning. Like
    /// the other degenerate shapes (empty `choose`, empty `repeat` body), reject
    /// it here so it can never reach the lift and invert its meaning.
    fn expect_count(&mut self) -> Result<u32, String> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::Number(n),
                start,
                ..
            }) => {
                let count = n.parse::<u32>().map_err(|_| {
                    self.error_at(start, "repeat count must be a non-negative integer")
                })?;
                if count == 0 {
                    return Err(self.error_at(start, "repeat count must be >= 1 (a `repeat 0` never runs its body and is not authorable as a loop)"));
                }
                Ok(count)
            }
            Some(tok) => Err(self.error_at(tok.start, "expected a repeat count (integer literal)")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_invariant(&mut self) -> Result<Invariant, String> {
        self.expect_keyword("invariant")?;
        let name = self.expect_ident()?;
        self.expect_symbol(';')?;
        Ok(match name.as_str() {
            "value_conserved" => Invariant::ValueConserved,
            "no_undeclared_state" => Invariant::NoUndeclaredState,
            other => Invariant::Custom(other.to_string()),
        })
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        let ident = self.expect_ident()?;
        Ok(match ident.as_str() {
            "int" => Type::Int,
            "bool" => Type::Bool,
            "pubkey" => Type::PubKey,
            "sig" => Type::Sig,
            "bytes32" => Type::Bytes32,
            "coin" => Type::Coin,
            "set" => {
                self.expect_symbol('<')?;
                let inner = self.parse_type()?;
                self.expect_symbol('>')?;
                Type::Set(Box::new(inner))
            }
            "map" => {
                self.expect_symbol('<')?;
                let key = self.parse_type()?;
                self.expect_symbol(',')?;
                let value = self.parse_type()?;
                self.expect_symbol('>')?;
                Type::Map(Box::new(key), Box::new(value))
            }
            other => Type::Named(other.to_string()),
        })
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), String> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::Ident(s),
                ..
            }) if s == keyword => Ok(()),
            Some(tok) => Err(self.error_at(tok.start, &format!("expected keyword {keyword}"))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::Ident(s),
                ..
            }) => Ok(s),
            Some(tok) => Err(self.error_at(tok.start, "expected identifier")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn peek_ident(&self) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Ident(_)))
    }

    fn expect_symbol(&mut self, symbol: char) -> Result<(), String> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::Symbol(c),
                ..
            }) if c == symbol => Ok(()),
            Some(tok) => Err(self.error_at(tok.start, &format!("expected '{symbol}'"))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn peek_symbol(&self, symbol: char) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Symbol(c)) if *c == symbol)
    }

    fn peek_keyword(&self, keyword: &str) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Ident(s)) if s == keyword)
    }

    fn expect_arrow(&mut self) -> Result<(), String> {
        match self.bump() {
            Some(Token {
                kind: TokenKind::Arrow,
                ..
            }) => Ok(()),
            Some(tok) => Err(self.error_at(tok.start, "expected '->'")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn consume_until(&mut self, symbol: char) -> Result<(), String> {
        while let Some(tok) = self.peek() {
            if matches!(tok.kind, TokenKind::Symbol(c) if c == symbol) {
                return Ok(());
            }
            self.pos += 1;
        }
        Err(self.error(&format!("expected '{symbol}'")))
    }

    fn consume_until_symbol(&mut self, symbol: char) -> Result<(), String> {
        self.consume_until(symbol)
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.src[start..end].trim().to_string()
    }

    fn prev_end(&self) -> usize {
        self.tokens
            .get(self.pos.saturating_sub(1))
            .map(|t| t.end)
            .unwrap_or(0)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn error(&self, msg: &str) -> String {
        if let Some(tok) = self.peek() {
            self.error_at(tok.start, msg)
        } else {
            msg.to_string()
        }
    }

    fn error_at(&self, pos: usize, msg: &str) -> String {
        format!("{msg} at byte {pos}")
    }
}

fn tokenize(src: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    let bytes = src.as_bytes();

    while i < bytes.len() {
        let c = src[i..].chars().next().unwrap();
        let c_len = c.len_utf8();

        if c.is_whitespace() {
            i += c_len;
            continue;
        }

        if c == '/' && src[i + 1..].starts_with('/') {
            i += 2;
            while i < bytes.len() {
                let ch = src[i..].chars().next().unwrap();
                if ch == '\n' {
                    break;
                }
                i += ch.len_utf8();
            }
            continue;
        }

        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += c_len;
            while i < bytes.len() {
                let ch = src[i..].chars().next().unwrap();
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Ident(src[start..i].to_string()),
                start,
                end: i,
            });
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            i += c_len;
            while i < bytes.len() {
                let ch = src[i..].chars().next().unwrap();
                if ch.is_ascii_digit() {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Number(src[start..i].to_string()),
                start,
                end: i,
            });
            continue;
        }

        let start = i;
        let token = match c {
            '-' if src[i + c_len..].starts_with('>') => {
                i += c_len + 1;
                TokenKind::Arrow
            }
            '<' if src[i + c_len..].starts_with('=') => {
                i += c_len + 1;
                TokenKind::Le
            }
            '>' if src[i + c_len..].starts_with('=') => {
                i += c_len + 1;
                TokenKind::Ge
            }
            '=' if src[i + c_len..].starts_with('=') => {
                i += c_len + 1;
                TokenKind::EqEq
            }
            '!' if src[i + c_len..].starts_with('=') => {
                i += c_len + 1;
                TokenKind::Ne
            }
            '&' if src[i + c_len..].starts_with('&') => {
                i += c_len + 1;
                TokenKind::AndAnd
            }
            '|' if src[i + c_len..].starts_with('|') => {
                i += c_len + 1;
                TokenKind::OrOr
            }
            other => {
                i += c_len;
                TokenKind::Symbol(other)
            }
        };
        tokens.push(Token {
            kind: token,
            start,
            end: i,
        });
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_counter_program_shape() {
        let src = include_str!("../../../../examples/counter.portrait");
        let program = parse(src).expect("counter.portrait should parse");

        assert_eq!(program.pragma, "portrait ^0.1.0");
        assert_eq!(program.uses.len(), 0);
        assert_eq!(program.app.name, "Counter");
        assert_eq!(program.app.roles.len(), 1);
        assert_eq!(program.app.lifecycle.len(), 1);
        assert!(program.app.flow.is_none());
        assert_eq!(program.app.invariants.len(), 1);
    }

    #[test]
    fn app_composition_grammar_yields_clear_diagnostic() {
        // An app-composition source (`contract <name> = <Type> { ... }`) is NOT
        // a covenant source. The parser must reject it with an actionable
        // message naming the covenant grammar, not a cryptic byte offset.
        let src = r#"pragma portrait ^0.1.0;
use custody::TimeVault;
app PersonalVault {
    contract vault = TimeVault {
        owner = param pubkey,
    }
}
"#;
        let err = parse(src).expect_err("app-composition must be rejected");
        assert!(
            err.contains("app-composition sources")
                && err.contains("role/lifecycle/flow/invariant"),
            "diagnostic should name the covenant grammar, got: {err}"
        );
    }

    /// Helper: fetch the single entrypoint body of the first role.
    fn first_entry_body(program: &Program) -> &[Stmt] {
        &program.app.roles[0].entrypoints[0].body
    }

    fn var(name: &str) -> Expr {
        Expr::Var(name.to_string())
    }

    fn bin(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    #[test]
    fn counter_return_parses_to_scalar_binary() {
        let src = include_str!("../../../../examples/counter.portrait");
        let program = parse(src).expect("counter.portrait should parse");
        let body = first_entry_body(&program);
        assert_eq!(body.len(), 1, "bump has exactly one statement");
        // return value + delta;
        assert_eq!(
            body[0],
            Stmt::Return(ReturnExpr::Scalar(bin(
                BinOp::Add,
                var("value"),
                var("delta")
            )))
        );
        // No Raw fallback anywhere.
        assert!(!body.iter().any(|s| matches!(s, Stmt::Raw(_))));
    }

    #[test]
    fn compliance_token_returns_parse_to_subtraction() {
        let src = include_str!("../../../../examples/tier3-demo/ComplianceToken.portrait");
        let program = parse(src).expect("ComplianceToken.portrait should parse");
        // Both transfer and verify_compliance bodies: `return balance - amount;`
        let expected = Stmt::Return(ReturnExpr::Scalar(bin(
            BinOp::Sub,
            var("balance"),
            var("amount"),
        )));
        for entry in &program.app.roles[0].entrypoints {
            assert_eq!(entry.body.len(), 1);
            assert_eq!(entry.body[0], expected);
            assert!(!entry.body.iter().any(|s| matches!(s, Stmt::Raw(_))));
        }
    }

    #[test]
    fn evidence_lineage_parses_requires_and_object_return() {
        let src = include_str!("../../../../library/attestation/EvidenceLineage.portrait");
        let program = parse(src).expect("EvidenceLineage.portrait should parse");
        let body = first_entry_body(&program);

        // 4 requires + 1 return, no Raw fallback.
        let requires: Vec<&Expr> = body
            .iter()
            .filter_map(|s| match s {
                Stmt::Require(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(requires.len(), 4, "attest has 4 require guards");
        assert!(
            !body.iter().any(|s| matches!(s, Stmt::Raw(_))),
            "no construct should fall back to Raw"
        );

        // requires checkSig(auth, issuer);  → Call with two Var args
        assert_eq!(
            *requires[0],
            Expr::Call {
                name: "checkSig".to_string(),
                args: vec![var("auth"), var("issuer")],
            }
        );
        // requires next_class >= 0;
        assert_eq!(
            *requires[1],
            bin(BinOp::Ge, var("next_class"), Expr::Int(0))
        );
        // requires next_t_bucket >= t_bucket;
        assert_eq!(
            *requires[2],
            bin(BinOp::Ge, var("next_t_bucket"), var("t_bucket"))
        );
        // requires next_t_bucket <= t_bucket + window;  (precedence: + binds tighter than <=)
        assert_eq!(
            *requires[3],
            bin(
                BinOp::Le,
                var("next_t_bucket"),
                bin(BinOp::Add, var("t_bucket"), var("window"))
            )
        );

        // return EvidenceLineage { seq: seq + 1, subject: subject, ... };
        let ret = body
            .iter()
            .find_map(|s| match s {
                Stmt::Return(r) => Some(r),
                _ => None,
            })
            .expect("attest has a return");
        match ret {
            ReturnExpr::Object { name, fields } => {
                assert_eq!(name.as_deref(), Some("EvidenceLineage"));
                assert_eq!(fields.len(), 5);
                assert_eq!(fields[0].0, "seq");
                assert_eq!(fields[0].1, bin(BinOp::Add, var("seq"), Expr::Int(1)));
                assert_eq!(fields[1], ("subject".to_string(), var("subject")));
            }
            other => panic!("expected object literal return, got {other:?}"),
        }
    }

    #[test]
    fn opinput_covenant_id_call_parses() {
        // From DigitalReit: requires parent_kov_id == OpInputCovenantId(0);
        let expr = parse_expr("parent_kov_id == OpInputCovenantId(0)").unwrap();
        assert_eq!(
            expr,
            bin(
                BinOp::Eq,
                var("parent_kov_id"),
                Expr::Call {
                    name: "OpInputCovenantId".to_string(),
                    args: vec![Expr::Int(0)],
                }
            )
        );
    }

    #[test]
    fn precedence_climbing_orders_operators() {
        // a || b && c == d + e * f  →  a || (b && (c == (d + (e * f))))
        let expr = parse_expr("a || b && c == d + e * f").unwrap();
        let expected = bin(
            BinOp::Or,
            var("a"),
            bin(
                BinOp::And,
                var("b"),
                bin(
                    BinOp::Eq,
                    var("c"),
                    bin(BinOp::Add, var("d"), bin(BinOp::Mul, var("e"), var("f"))),
                ),
            ),
        );
        assert_eq!(expr, expected);
    }

    #[test]
    fn unary_and_postfix_parse() {
        // -a, !b, foo.bar, arr[0]
        assert_eq!(
            parse_expr("-a").unwrap(),
            Expr::Unary {
                op: UnOp::Neg,
                rhs: Box::new(var("a"))
            }
        );
        assert_eq!(
            parse_expr("!b").unwrap(),
            Expr::Unary {
                op: UnOp::Not,
                rhs: Box::new(var("b"))
            }
        );
        assert_eq!(
            parse_expr("prev_states[0].seq").unwrap(),
            Expr::Field {
                base: Box::new(Expr::Index {
                    base: Box::new(var("prev_states")),
                    index: Box::new(Expr::Int(0)),
                }),
                field: "seq".to_string(),
            }
        );
    }

    #[test]
    fn to_silverscript_roundtrips_canonically() {
        // Scalar
        let r = parse_return_expr("value + delta").unwrap();
        assert_eq!(r.to_silverscript(), "value + delta");
        // Object literal
        let r = parse_return_expr("Name { a: x + 1, b: y }").unwrap();
        assert_eq!(r.to_silverscript(), "Name { a: x + 1, b: y }");
        // Call inside comparison
        let e = parse_expr("parent_kov_id == OpInputCovenantId(0)").unwrap();
        assert_eq!(e.to_silverscript(), "parent_kov_id == OpInputCovenantId(0)");
    }

    #[test]
    fn raw_fallback_on_unparseable_require() {
        // A require with a trailing garbage token the expression grammar cannot
        // consume must fall back to Stmt::Raw rather than erroring the parse.
        let src = r#"pragma portrait ^0.1.0;
app A {
  role r {
    state { int v; }
    #[covenant(mode = transition)]
    entrypoint function f() : (int v) {
      requires v @ 1;
      return v + 1;
    }
  }
  lifecycle { live -> live via r.f; }
  invariant no_undeclared_state;
}
"#;
        let program = parse(src).expect("program with a raw require still parses");
        let body = first_entry_body(&program);
        // The require fell back to Raw; the return parsed normally.
        assert!(
            matches!(&body[0], Stmt::Raw(s) if s.contains("v @ 1")),
            "first stmt should be Raw fallback, got {:?}",
            body[0]
        );
        assert_eq!(
            body[1],
            Stmt::Return(ReturnExpr::Scalar(bin(BinOp::Add, var("v"), Expr::Int(1))))
        );
    }

    // ── Rejection set (fail-loud, names the construct, routes to vProgs) ──────

    /// Build a minimal program whose single entrypoint body is `body_stmt`.
    fn program_with_body(body_stmt: &str) -> String {
        format!(
            r#"pragma portrait ^0.1.0;
app A {{
  role r {{
    state {{ int v; }}
    #[covenant(mode = transition)]
    entrypoint function f() : (int v) {{
      {body_stmt}
      return v + 1;
    }}
  }}
  lifecycle {{ live -> live via r.f; }}
  invariant no_undeclared_state;
}}
"#
        )
    }

    #[test]
    fn rejects_unbounded_loop_naming_construct() {
        let src = program_with_body("for (i = 0; i < n; i = i + 1) { x = x + 1 };");
        let err = parse(&src).expect_err("unbounded loop must be rejected");
        assert!(
            err.contains("unsupported construct `for`")
                && err.contains("vProgs layer")
                && err.contains("SOLIDITY-SUBSET-V0 §3"),
            "diagnostic must name the construct + vProgs route, got: {err}"
        );
    }

    #[test]
    fn rejects_event_emit_naming_construct() {
        let src = program_with_body("emit Transfer(from, to, amount);");
        let err = parse(&src).expect_err("emit must be rejected");
        assert!(
            err.contains("unsupported construct `emit`") && err.contains("event logs"),
            "diagnostic must name emit + event logs, got: {err}"
        );
    }

    #[test]
    fn rejects_cross_contract_call_naming_construct() {
        let src = program_with_body("target.call(payload);");
        let err = parse(&src).expect_err("synchronous cross-contract call must be rejected");
        assert!(
            err.contains("unsupported construct `call`")
                && err.contains("cross-contract")
                && err.contains("vProgs layer"),
            "diagnostic must name the call + cross-contract reason, got: {err}"
        );
    }

    #[test]
    fn rejects_allowance_transfer_from_naming_construct() {
        let src = program_with_body("transferFrom(owner, to, amount);");
        let err = parse(&src).expect_err("transferFrom must be rejected");
        assert!(
            err.contains("unsupported construct `transferFrom`") && err.contains("allowance"),
            "diagnostic must name transferFrom + allowance reason, got: {err}"
        );
    }

    #[test]
    fn rejects_mapping_state_field_naming_construct() {
        let src = r#"pragma portrait ^0.1.0;
app A {
  role r {
    state { map<pubkey, int> balances; }
    #[covenant(mode = transition)]
    entrypoint function f() : (int v) {
      return v + 1;
    }
  }
  lifecycle { live -> live via r.f; }
  invariant no_undeclared_state;
}
"#;
        let err = parse(src).expect_err("mapping state field must be rejected");
        assert!(
            err.contains("unsupported construct `map<K, V>`")
                && err.contains("shared mutable mappings")
                && err.contains("vProgs layer"),
            "diagnostic must name the mapping + vProgs route, got: {err}"
        );
    }

    // ── Embedded-vector rejection (blacklisted construct inside require/return) ──
    //
    // Regression probes for the adversarial-verify finding: a blacklisted
    // construct embedded inside a `require`/`return` was bypassing the fail-loud
    // rejection set, silently degrading to `Stmt::Raw` (FALSE ACCEPT) instead of
    // loud-rejecting as the standalone form does.

    #[test]
    fn rejects_cross_contract_call_embedded_in_require() {
        let src = program_with_body("require strategy.call(amount);");
        let err = parse(&src).expect_err(
            "a blacklisted .call embedded in require must be rejected, not degraded to Raw",
        );
        assert!(
            err.contains("unsupported construct `call`")
                && err.contains("cross-contract")
                && err.contains("vProgs layer"),
            "embedded require must name the construct identically to the standalone path, got: {err}"
        );
    }

    #[test]
    fn rejects_cross_contract_call_embedded_in_return() {
        let src = program_with_body("return strategy.call(amount);");
        let err = parse(&src).expect_err(
            "a blacklisted .call embedded in return must be rejected, not degraded to Raw",
        );
        assert!(
            err.contains("unsupported construct `call`")
                && err.contains("cross-contract")
                && err.contains("vProgs layer"),
            "embedded return must name the construct identically to the standalone path, got: {err}"
        );
    }

    #[test]
    fn rejects_cross_contract_call_embedded_in_return_object_field() {
        // Deepest embedding: a blacklisted construct nested in an object-literal
        // return field. Must still loud-reject naming the construct.
        let src = program_with_body("return State { next: target.call(payload) };");
        let err = parse(&src)
            .expect_err("a blacklisted .call nested in a return object field must be rejected");
        assert!(
            err.contains("unsupported construct `call`")
                && err.contains("cross-contract")
                && err.contains("vProgs layer"),
            "nested object-field return must name the construct, got: {err}"
        );
    }

    #[test]
    fn rejection_set_does_not_swallow_unrelated_raw_forms() {
        // A non-blacklisted unknown statement form must STILL fall back to Raw,
        // not be hijacked by the rejection check (boundary is precise).
        let src = program_with_body("v @ 1;");
        let program = parse(&src).expect("non-blacklisted raw form still parses");
        let body = first_entry_body(&program);
        assert!(
            matches!(&body[0], Stmt::Raw(s) if s.contains("v @ 1")),
            "non-blacklisted form should be Raw, got {:?}",
            body[0]
        );
    }

    /// Build a minimal program with ONE covenant entrypoint (`settle`) and ONE
    /// attribute-less vProg entrypoint (`tally`) whose body is `vprog_stmt`. The
    /// vProg entrypoint is NOT on the lifecycle (it is off-L1), mirroring the
    /// tier3-demo layout.
    fn program_with_vprog_body(vprog_stmt: &str) -> String {
        format!(
            r#"pragma portrait ^0.1.0;
app A {{
  role r {{
    state {{ int v; }}
    #[covenant(mode = transition)]
    entrypoint function settle(int amount) : (int v) {{
      return v - amount;
    }}
    entrypoint function tally(int n) {{
      {vprog_stmt}
      return v;
    }}
  }}
  lifecycle {{ live -> live via r.settle; }}
  invariant no_undeclared_state;
}}
"#
        )
    }

    #[test]
    fn rejection_set_construct_allowed_in_noncovenant_entrypoint() {
        // The CENTRAL allocation fix: a REJECTION_SET construct (here a `for`
        // loop) placed in an attribute-less (NonCovenant / vProg) entrypoint must
        // NOT be rejected at parse — vProgs are exactly where loops belong. It is
        // degraded to a recorded `Stmt::Raw` hole that survives to the IR/sema.
        let src = program_with_vprog_body("for (i = 0; i < n; i = i + 1) { x = x + 1 };");
        let program =
            parse(&src).expect("a `for` loop in a vProg entrypoint must parse, not be rejected");
        // tally is the second entrypoint; its first stmt is the loop, carried Raw.
        let tally = &program.app.roles[0].entrypoints[1];
        assert!(matches!(tally.mode, CovenantMode::NonCovenant));
        assert!(
            matches!(&tally.body[0], Stmt::Raw(s) if s.contains("for") && s.contains("x = x + 1")),
            "the loop should be a recorded Raw hole, got {:?}",
            tally.body[0]
        );
        // The trailing `return v;` after the loop still parses normally.
        assert_eq!(tally.body[1], Stmt::Return(ReturnExpr::Scalar(var("v"))));
    }

    #[test]
    fn rejection_set_construct_still_rejected_in_covenant_entrypoint() {
        // The covenant path stays byte-identical: the SAME `for` loop inside a
        // `#[covenant]` entrypoint is still loud-rejected, naming the construct.
        let src = program_with_body("for (i = 0; i < n; i = i + 1) { x = x + 1 };");
        let err = parse(&src).expect_err("a `for` loop in a covenant entrypoint must be rejected");
        assert!(
            err.contains("unsupported construct `for`") && err.contains("vProgs layer"),
            "covenant-mode rejection must be unchanged, got: {err}"
        );
    }

    #[test]
    fn rejection_set_method_call_allowed_in_noncovenant_entrypoint() {
        // A method-call rejection-set form (`.call(`) embedded in a vProg body is
        // likewise degraded to Raw, not rejected.
        let src = program_with_vprog_body("oracle.call(payload);");
        let program = parse(&src).expect("a `.call` in a vProg entrypoint must parse");
        let tally = &program.app.roles[0].entrypoints[1];
        assert!(
            matches!(&tally.body[0], Stmt::Raw(s) if s.contains("oracle.call")),
            "the cross-call should be a recorded Raw hole, got {:?}",
            tally.body[0]
        );
    }

    // ----- flow control-construct surface syntax (M4) ------------------------

    /// Wrap a `flow {}` body in a minimal parseable covenant program and return
    /// the parsed flow steps.
    fn parse_flow_steps(flow_body: &str) -> Vec<Step> {
        let src = format!("pragma portrait ^0.1.0;\napp F {{\n  flow {{\n{flow_body}\n  }}\n}}\n");
        let program = parse(&src).expect("flow program should parse");
        program.app.flow.expect("flow block must be present").steps
    }

    #[test]
    fn move_only_flow_still_parses() {
        // No regression: the existing `<role>.<entry>` Move form is unchanged.
        let steps = parse_flow_steps("a.first; b.second");
        assert_eq!(steps.len(), 2);
        assert!(matches!(&steps[0], Step::Move { role, entry } if role == "a" && entry == "first"));
        assert!(
            matches!(&steps[1], Step::Move { role, entry } if role == "b" && entry == "second")
        );
    }

    #[test]
    fn choose_parses_to_choose_with_branches() {
        let steps = parse_flow_steps("choose {\n  branch { a.yes }\n  branch { b.no }\n}");
        assert_eq!(steps.len(), 1);
        let Step::Choose(branches) = &steps[0] else {
            panic!("expected Step::Choose, got {:?}", steps[0]);
        };
        assert_eq!(branches.len(), 2);
        assert!(
            matches!(&branches[0].steps[0], Step::Move { role, entry } if role == "a" && entry == "yes")
        );
        assert!(
            matches!(&branches[1].steps[0], Step::Move { role, entry } if role == "b" && entry == "no")
        );
    }

    #[test]
    fn par_parses_to_par_with_threads() {
        let steps = parse_flow_steps("par {\n  thread { a.x }\n  thread { b.y }\n}");
        assert_eq!(steps.len(), 1);
        let Step::Par(threads) = &steps[0] else {
            panic!("expected Step::Par, got {:?}", steps[0]);
        };
        assert_eq!(threads.len(), 2);
        assert!(
            matches!(&threads[0].steps[0], Step::Move { role, entry } if role == "a" && entry == "x")
        );
        assert!(
            matches!(&threads[1].steps[0], Step::Move { role, entry } if role == "b" && entry == "y")
        );
    }

    #[test]
    fn repeat_parses_to_repeat_with_count_and_body() {
        let steps = parse_flow_steps("repeat 3 { a.tick; b.tock }");
        assert_eq!(steps.len(), 1);
        let Step::Repeat(count, body) = &steps[0] else {
            panic!("expected Step::Repeat, got {:?}", steps[0]);
        };
        assert_eq!(*count, 3);
        assert_eq!(body.steps.len(), 2);
        assert!(
            matches!(&body.steps[0], Step::Move { role, entry } if role == "a" && entry == "tick")
        );
    }

    #[test]
    fn nested_control_constructs_parse() {
        // A repeat whose body holds a choose: nesting reuses the same step grammar.
        let steps = parse_flow_steps(
            "repeat 2 {\n  choose {\n    branch { a.l }\n    branch { b.r }\n  }\n}",
        );
        let Step::Repeat(2, body) = &steps[0] else {
            panic!("expected outer Step::Repeat(2, ..), got {:?}", steps[0]);
        };
        assert!(matches!(&body.steps[0], Step::Choose(b) if b.len() == 2));
    }

    #[test]
    fn empty_choose_is_a_parse_error() {
        let src = "pragma portrait ^0.1.0;\napp F { flow { choose {} } }\n";
        let err = parse(src).expect_err("an empty choose must not parse");
        assert!(
            err.contains("at least one `branch"),
            "error should name the missing branch, got: {err}"
        );
    }

    #[test]
    fn bad_branch_label_is_a_parse_error() {
        // `choose { foo { .. } }` uses the wrong label keyword; must fail loudly,
        // not silently mis-parse `foo` as a Move role.
        let src = "pragma portrait ^0.1.0;\napp F { flow { choose { foo { a.x } } } }\n";
        let err = parse(src).expect_err("a mislabelled branch must not parse");
        assert!(
            err.contains("expected keyword branch"),
            "error should name the expected branch keyword, got: {err}"
        );
    }

    #[test]
    fn unterminated_choose_is_a_parse_error() {
        let src = "pragma portrait ^0.1.0;\napp F { flow { choose { branch { a.x } ";
        parse(src).expect_err("an unterminated choose must not parse");
    }

    #[test]
    fn repeat_without_count_is_a_parse_error() {
        let src = "pragma portrait ^0.1.0;\napp F { flow { repeat { a.x } } }\n";
        let err = parse(src).expect_err("a repeat with no count must not parse");
        assert!(
            err.contains("repeat count"),
            "error should name the missing count, got: {err}"
        );
    }

    #[test]
    fn repeat_zero_is_a_parse_error() {
        // `repeat 0` means "run the body zero times" in the source, but the M2
        // lift models any `repeat` as an UNBOUNDED `μX.(body.X)` loop — so a
        // `repeat 0` would silently lift to the *opposite* meaning (an infinite
        // loop). Mirror the other degenerate-shape rejections: a loop that never
        // runs is not authorable as a loop, so reject it at parse time with a
        // named error rather than let it reach the lift and invert its meaning.
        let src = "pragma portrait ^0.1.0;\napp F { flow { repeat 0 { a.x } } }\n";
        let err = parse(src).expect_err("a repeat 0 must not parse");
        assert!(
            err.contains("repeat count must be >= 1"),
            "error should name the >= 1 requirement, got: {err}"
        );
    }

    #[test]
    fn rejection_set_table_is_nonempty_and_well_formed() {
        // Single-source-of-truth sanity: every entry names a construct, a reason,
        // and a SUBSET-V0 §3 reference so prose and code cannot drift unnoticed.
        assert!(!REJECTION_SET.is_empty());
        for rc in REJECTION_SET {
            assert!(!rc.lead.is_empty(), "construct lead must be named");
            assert!(!rc.reason.is_empty(), "construct must carry a reason");
            assert!(
                rc.subset_ref.contains("SOLIDITY-SUBSET-V0 §3"),
                "each entry must cite SOLIDITY-SUBSET-V0 §3"
            );
        }
    }
}
