//! SMT-LIB document assembly. Collects declarations and assertions, then renders
//! a complete `(set-logic ...) ... (check-sat) (get-model)` script.

/// Incrementally builds an SMT-LIB document. Declarations are de-duplicated and
/// the opaque sorts they reference are declared once at the top.
#[derive(Clone)]
pub struct SmtBuilder {
    decls: Vec<String>,
    asserts: Vec<String>,
    /// Opaque sorts referenced by any declaration (declared with `declare-sort`).
    needs_sort: std::collections::BTreeSet<String>,
}

impl SmtBuilder {
    /// A fresh, empty builder.
    pub fn new() -> Self {
        Self {
            decls: Vec::new(),
            asserts: Vec::new(),
            needs_sort: std::collections::BTreeSet::new(),
        }
    }

    /// Declare a constant of the given sort. If the sort is an opaque
    /// (uninterpreted) sort it is recorded for a `declare-sort` line.
    pub fn declare_const(&mut self, name: &str, sort: &str) {
        if is_opaque_sort(sort) {
            self.needs_sort.insert(sort.to_string());
        }
        let line = format!("(declare-const {name} {sort})");
        if !self.decls.contains(&line) {
            self.decls.push(line);
        }
    }

    /// Declare an uninterpreted function `(declare-fun name (args) ret)`. Opaque
    /// argument/result sorts are recorded for `declare-sort` lines.
    pub fn declare_fun(&mut self, name: &str, args: &[String], ret: &str) {
        for s in args {
            if is_opaque_sort(s) {
                self.needs_sort.insert(s.clone());
            }
        }
        if is_opaque_sort(ret) {
            self.needs_sort.insert(ret.to_string());
        }
        let line = format!("(declare-fun {name} ({}) {ret})", args.join(" "));
        if !self.decls.contains(&line) {
            self.decls.push(line);
        }
    }

    /// Add an assertion (the term is the body of `(assert ...)`).
    pub fn assert(&mut self, term: &str) {
        self.asserts.push(format!("(assert {term})"));
    }

    /// Render the full document with the given logic name. Includes `get-model`
    /// so a `sat` (REFUTED) result carries a concrete counter-example.
    ///
    /// Borrows `&self` so the same builder can be rendered more than once — e.g.
    /// once with only the transition relation `T` asserted (the vacuity probe)
    /// and again after the negated VC is added (the full conservation query).
    pub fn finish(&self, logic: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!("(set-logic {logic})\n"));
        for s in &self.needs_sort {
            out.push_str(&format!("(declare-sort {s} 0)\n"));
        }
        for d in &self.decls {
            out.push_str(d);
            out.push('\n');
        }
        for a in &self.asserts {
            out.push_str(a);
            out.push('\n');
        }
        out.push_str("(check-sat)\n");
        out.push_str("(get-model)\n");
        out
    }
}

impl Default for SmtBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque (uninterpreted) sorts that require a `declare-sort` line. `Int`/`Bool`
/// are built-in and need none.
fn is_opaque_sort(sort: &str) -> bool {
    !matches!(sort, "Int" | "Bool")
}
