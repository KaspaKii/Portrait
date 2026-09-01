//! Load a committed silverc covenant artifact and re-derive its per-state
//! script without a compiler.
//!
//! A silverc-compiled covenant script is laid out as
//! `<head> <state region> <program body>`; the artifact records where the
//! state region sits (`state_layout`). Every UTXO of a covenant chain runs the
//! *same* program body and differs only inside that region, so a per-state
//! script can be produced from the committed artifact by splicing new state
//! bytes in — no `silverscript-lang` dependency (which would float the engine
//! pin), and no secret material.
//!
//! The state region uses **explicit length-prefixed pushes**
//! ([`push_state_field`]) so the layout stays fixed-width; the arguments in a
//! spending signature script use the **canonical** encoding
//! (`ScriptBuilder::add_data`, which folds single small bytes into `OP_N`).
//! The two are not interchangeable.
//!
//! Status: **v0 — unaudited — testnet first.**

use std::path::Path;

use kaspa_txscript::{script_builder::ScriptBuilder, EngineFlags};

use crate::error::{Error, Result};

/// `OP_0` — selects the leader entry point (index 0 of the artifact ABI) in a
/// covenant-declaration signature script.
const LEADER_SELECTOR: u8 = 0x00;

/// A compiled covenant artifact (`*.compiled.json` as emitted by silverc),
/// loaded as data.
pub struct CompiledCovenant {
    /// The full compiled script, exactly as committed.
    pub script: Vec<u8>,
    /// Offset of the state region within [`Self::script`].
    pub state_start: usize,
    /// Length of the state region, in bytes.
    pub state_len: usize,
}

impl CompiledCovenant {
    /// Load an artifact from a committed `*.compiled.json`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::ConditionInvalid(format!("read {}: {e}", path.display())))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| Error::ConditionInvalid(format!("parse {}: {e}", path.display())))?;

        let script: Vec<u8> = serde_json::from_value(json["script"].clone())
            .map_err(|e| Error::ConditionInvalid(format!("artifact `script`: {e}")))?;
        let state_start = usize_at(&json, "state_layout", "start")?;
        let state_len = usize_at(&json, "state_layout", "len")?;

        let state_end = state_start.checked_add(state_len).ok_or_else(|| {
            Error::ConditionInvalid(format!(
                "state_layout {{{state_start}, {state_len}}} overflows"
            ))
        })?;
        if state_end > script.len() {
            return Err(Error::ConditionInvalid(format!(
                "state_layout {{{state_start}, {state_len}}} runs past the {}-byte script",
                script.len()
            )));
        }
        Ok(Self {
            script,
            state_start,
            state_len,
        })
    }

    /// The program body: everything after the state region. Identical across
    /// every state of a covenant chain.
    pub fn program_body(&self) -> &[u8] {
        &self.script[self.state_start + self.state_len..]
    }

    /// The committed script with its state region replaced by `state`.
    ///
    /// Errors if `state` is not exactly [`Self::state_len`] bytes — a
    /// wrong-width state would shift the program body and silently produce a
    /// different covenant.
    pub fn try_with_state(&self, state: &[u8]) -> Result<Vec<u8>> {
        if state.len() != self.state_len {
            return Err(Error::ConditionInvalid(format!(
                "state region must be exactly {} bytes, got {}",
                self.state_len,
                state.len()
            )));
        }
        let mut script = self.script.clone();
        script[self.state_start..self.state_start + self.state_len].copy_from_slice(state);
        Ok(script)
    }

    /// [`Self::try_with_state`], panicking on a wrong-width state. Convenience
    /// for tests and for callers that construct the state themselves.
    ///
    /// # Panics
    /// If `state` is not exactly [`Self::state_len`] bytes.
    pub fn with_state(&self, state: &[u8]) -> Vec<u8> {
        self.try_with_state(state).expect("state region width")
    }
}

/// Append one state-region field to `out` using an explicit length-prefixed
/// push (never the `OP_N` small-integer form), matching the fixed-width layout
/// silverc emits.
///
/// Errors if `field` is not a 1..=75 byte direct push (no state field in this
/// library's covenants is outside that range).
pub fn push_state_field(out: &mut Vec<u8>, field: &[u8]) -> Result<()> {
    if field.is_empty() || field.len() > 75 {
        return Err(Error::ConditionInvalid(format!(
            "state field must be a 1..=75 byte direct push, got {}",
            field.len()
        )));
    }
    out.push(field.len() as u8);
    out.extend_from_slice(field);
    Ok(())
}

/// Assemble the leader signature script for a covenant-declaration spend:
/// `<state pushes> <sig> <trailing pushes> OP_0 <redeem>`.
///
/// `state_pushes` and `trailing_pushes` are pre-encoded argument pushes (the
/// canonical `ScriptBuilder` encoding); `sig_65` is a 64-byte Schnorr
/// signature plus its sighash-type byte; `redeem` is the covenant script being
/// spent.
pub fn append_signature_script(
    state_pushes: &[u8],
    sig_65: &[u8],
    trailing_pushes: &[u8],
    redeem: &[u8],
) -> Result<Vec<u8>> {
    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    builder
        .add_data(sig_65)
        .map_err(|e| Error::ConditionInvalid(format!("push signature: {e}")))?;
    let sig_push = builder.drain().to_vec();

    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    builder
        .add_data(redeem)
        .map_err(|e| Error::ConditionInvalid(format!("push redeem script: {e}")))?;
    let redeem_push = builder.drain().to_vec();

    let mut script = Vec::with_capacity(
        state_pushes.len() + sig_push.len() + trailing_pushes.len() + 1 + redeem_push.len(),
    );
    script.extend_from_slice(state_pushes);
    script.extend_from_slice(&sig_push);
    script.extend_from_slice(trailing_pushes);
    script.push(LEADER_SELECTOR);
    script.extend_from_slice(&redeem_push);
    Ok(script)
}

fn usize_at(json: &serde_json::Value, object: &str, key: &str) -> Result<usize> {
    json[object][key]
        .as_u64()
        .map(|v| v as usize)
        .ok_or_else(|| Error::ConditionInvalid(format!("artifact `{object}.{key}` missing")))
}

#[cfg(test)]
mod tests {
    use super::{push_state_field, CompiledCovenant};

    fn covenant() -> CompiledCovenant {
        CompiledCovenant {
            script: (0u8..20).collect(),
            state_start: 4,
            state_len: 6,
        }
    }

    #[test]
    fn with_state_rewrites_only_the_state_window() {
        let cov = covenant();
        let spliced = cov.with_state(&[0xff; 6]);
        assert_eq!(spliced.len(), cov.script.len());
        assert_eq!(&spliced[..4], &cov.script[..4]);
        assert_eq!(&spliced[4..10], &[0xff; 6]);
        assert_eq!(&spliced[10..], &cov.script[10..]);
    }

    #[test]
    fn program_body_starts_after_the_state_window() {
        let cov = covenant();
        assert_eq!(cov.program_body(), &cov.script[10..]);
    }

    #[test]
    fn try_with_state_rejects_a_wrong_width_state() {
        let cov = covenant();
        assert!(cov.try_with_state(&[0xff; 5]).is_err());
        assert!(cov.try_with_state(&[0xff; 7]).is_err());
        assert!(cov.try_with_state(&[0xff; 6]).is_ok());
    }

    #[test]
    fn push_state_field_uses_an_explicit_length_prefix() {
        let mut out = Vec::new();
        push_state_field(&mut out, &[0x01]).expect("1-byte field");
        assert_eq!(out, vec![0x01, 0x01], "must not fold into OP_1");
    }

    #[test]
    fn push_state_field_rejects_non_direct_pushes() {
        let mut out = Vec::new();
        assert!(push_state_field(&mut out, &[]).is_err());
        assert!(push_state_field(&mut out, &[0u8; 76]).is_err());
    }
}
