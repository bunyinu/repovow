use anyhow::{bail, Context, Result};
use ed25519_dalek::{
    Signer as Ed25519Signer, SigningKey as Ed25519SigningKey, Verifier as Ed25519Verifier,
    VerifyingKey as Ed25519VerifyingKey,
};
use p256::ecdsa::signature::Verifier as P256Verifier;
use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey,
};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use crate::paths::{
    read_json, repovow_dir, utcnow, write_json_atomic, POLICY_KEY_FILE, POLICY_PUB_FILE,
    POLICY_SIG_FILE,
};
use crate::state::{load_config, load_state, log_event, save_config, Goal, PolicyMode};

const POLICY_MD_FILE: &str = "policy.md";
const POLICY_WARNING_MARKER_FILE: &str = "policy-warning-marker.json";
const POLICY_VERSION: u32 = 1;

/// Default for new `repovow policy init` — NIST P-256 / secp256r1 (FIPS 186-4 approved).
pub const DEFAULT_POLICY_ALGORITHM: &str = "ecdsa-p256";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyAlgorithm {
    /// ECDSA on NIST P-256 (secp256r1) — FIPS 186-4 approved algorithm.
    EcdsaP256,
    /// Legacy; verify-only for existing repos. Not a FIPS-approved algorithm.
    Ed25519,
}

impl PolicyAlgorithm {
    fn id(self) -> &'static str {
        match self {
            Self::EcdsaP256 => "ecdsa-p256",
            Self::Ed25519 => "ed25519",
        }
    }

    fn fips_note(self) -> &'static str {
        match self {
            Self::EcdsaP256 => "FIPS 186-4 approved algorithm (ECDSA P-256)",
            Self::Ed25519 => "not FIPS-approved — migrate with `repovow policy init`",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyStatus {
    Off,
    Valid {
        signed_at: String,
        algorithm: String,
    },
    Unsigned,
    Tampered,
    MissingPublicKey,
    InvalidSignature,
    NoGoal,
}

impl PolicyStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, PolicyStatus::Off | PolicyStatus::Valid { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            PolicyStatus::Off => "off",
            PolicyStatus::Valid { .. } => "valid",
            PolicyStatus::Unsigned => "unsigned",
            PolicyStatus::Tampered => "tampered",
            PolicyStatus::MissingPublicKey => "missing public key",
            PolicyStatus::InvalidSignature => "invalid signature",
            PolicyStatus::NoGoal => "no goal",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            PolicyStatus::Off => "Policy signing is disabled.".into(),
            PolicyStatus::Valid { signed_at, algorithm } => {
                format!("Signature valid ({algorithm}, signed {signed_at}).")
            }
            PolicyStatus::Unsigned => {
                "Goal has no signature. Run `repovow policy sign` or commit a valid policy.sig.".into()
            }
            PolicyStatus::Tampered => {
                "Goal fields do not match the signed policy — possible tampering or stale signature. \
                 Re-sign with `repovow policy sign` or restore from trusted policy.md."
                    .into()
            }
            PolicyStatus::MissingPublicKey => {
                "No policy.pub — run `repovow policy init` or `repovow policy trust <pubkey>`.".into()
            }
            PolicyStatus::InvalidSignature => "Signature does not verify against policy.pub.".into(),
            PolicyStatus::NoGoal => "No active goal to verify.".into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PolicyPayload {
    v: u32,
    title: String,
    acceptance: Vec<String>,
    constraints: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicySignature {
    v: u32,
    algorithm: String,
    signature: String,
    signed_at: String,
    payload_sha256: String,
}

#[derive(Debug, Clone)]
struct KeyMaterial {
    algorithm: PolicyAlgorithm,
    public_bytes: Vec<u8>,
    secret_bytes: Option<Vec<u8>>,
}

pub(crate) fn parse_algorithm(s: &str) -> Result<PolicyAlgorithm> {
    match s.trim().to_ascii_lowercase().as_str() {
        "ecdsa-p256" | "p256" | "secp256r1" | "prime256v1" => Ok(PolicyAlgorithm::EcdsaP256),
        "ed25519" => Ok(PolicyAlgorithm::Ed25519),
        other => bail!("unknown policy algorithm: {other} (use ecdsa-p256 or ed25519)"),
    }
}

pub fn canonical_payload(goal: &Goal) -> Result<Vec<u8>> {
    let payload = PolicyPayload {
        v: POLICY_VERSION,
        title: goal.title.trim().to_string(),
        acceptance: goal.acceptance.clone(),
        constraints: goal.constraints.clone(),
    };
    Ok(serde_json::to_vec(&payload)?)
}

fn payload_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    to_hex(&digest)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(hex: &str) -> Result<Vec<u8>> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        bail!("invalid hex length");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).context("invalid hex"))
        .collect()
}

fn policy_paths(
    root: Option<&Path>,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let dir = repovow_dir(root);
    (
        dir.join(POLICY_PUB_FILE),
        dir.join(POLICY_KEY_FILE),
        dir.join(POLICY_SIG_FILE),
    )
}

fn parse_key_file(content: &str) -> Result<(PolicyAlgorithm, Vec<u8>)> {
    let lines: Vec<&str> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match lines.as_slice() {
        [algo, hex] => Ok((parse_algorithm(algo)?, from_hex(hex)?)),
        [hex] => {
            let bytes = from_hex(hex)?;
            match bytes.len() {
                32 => Ok((PolicyAlgorithm::Ed25519, bytes)), // legacy policy.pub
                33 | 65 => Ok((PolicyAlgorithm::EcdsaP256, bytes)),
                64 => Ok((PolicyAlgorithm::Ed25519, bytes)), // legacy ed25519 keypair
                n => bail!("unexpected key material length: {n} bytes"),
            }
        }
        _ => bail!("invalid policy key file format"),
    }
}

fn write_key_file(path: &Path, algorithm: PolicyAlgorithm, hex: &str) -> Result<()> {
    fs::write(path, format!("{}\n{hex}\n", algorithm.id()))?;
    Ok(())
}

fn read_public_material(root: Option<&Path>) -> Result<KeyMaterial> {
    let (pub_path, _, _) = policy_paths(root);
    let content =
        fs::read_to_string(&pub_path).with_context(|| format!("read {}", pub_path.display()))?;
    let (algorithm, public_bytes) = parse_key_file(&content)?;
    validate_public_key(algorithm, &public_bytes)?;
    Ok(KeyMaterial {
        algorithm,
        public_bytes,
        secret_bytes: None,
    })
}

fn read_signing_material(root: Option<&Path>) -> Result<KeyMaterial> {
    let (_, key_path, _) = policy_paths(root);
    let content =
        fs::read_to_string(&key_path).with_context(|| format!("read {}", key_path.display()))?;
    let (algorithm, secret_bytes) = parse_key_file(&content)?;
    validate_secret_key(algorithm, &secret_bytes)?;
    Ok(KeyMaterial {
        algorithm,
        public_bytes: vec![],
        secret_bytes: Some(secret_bytes),
    })
}

fn validate_public_key(algorithm: PolicyAlgorithm, bytes: &[u8]) -> Result<()> {
    match algorithm {
        PolicyAlgorithm::EcdsaP256 => {
            P256VerifyingKey::from_sec1_bytes(bytes).context("invalid P-256 public key")?;
        }
        PolicyAlgorithm::Ed25519 => {
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("ed25519 policy.pub must be 32 bytes"))?;
            Ed25519VerifyingKey::from_bytes(&arr).context("invalid ed25519 public key")?;
        }
    }
    Ok(())
}

fn validate_secret_key(algorithm: PolicyAlgorithm, bytes: &[u8]) -> Result<()> {
    match algorithm {
        PolicyAlgorithm::EcdsaP256 => {
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("P-256 policy.key must be 32 bytes"))?;
            P256SigningKey::from_bytes(&arr.into()).context("invalid P-256 signing key")?;
        }
        PolicyAlgorithm::Ed25519 => {
            let arr: [u8; 64] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("ed25519 policy.key must be 64 bytes"))?;
            Ed25519SigningKey::from_keypair_bytes(&arr).context("invalid ed25519 signing key")?;
        }
    }
    Ok(())
}

fn sign_bytes(algorithm: PolicyAlgorithm, secret: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    match algorithm {
        PolicyAlgorithm::EcdsaP256 => {
            let arr: [u8; 32] = secret
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid P-256 secret length"))?;
            let key = P256SigningKey::from_bytes(&arr.into())?;
            let sig: P256Signature = key.sign(payload);
            Ok(sig.to_bytes().to_vec())
        }
        PolicyAlgorithm::Ed25519 => {
            let key = if secret.len() == 64 {
                let arr: [u8; 64] = secret.try_into().unwrap();
                Ed25519SigningKey::from_keypair_bytes(&arr)?
            } else {
                bail!("ed25519 signing requires 64-byte keypair in policy.key");
            };
            Ok(Ed25519Signer::sign(&key, payload).to_bytes().to_vec())
        }
    }
}

fn verify_bytes(
    algorithm: PolicyAlgorithm,
    public: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> Result<()> {
    let sig_arr: [u8; 64] = signature
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    match algorithm {
        PolicyAlgorithm::EcdsaP256 => {
            let key = P256VerifyingKey::from_sec1_bytes(public)?;
            let sig = P256Signature::from_bytes(&sig_arr.into())?;
            P256Verifier::verify(&key, payload, &sig)
                .map_err(|_| anyhow::anyhow!("P-256 signature mismatch"))
        }
        PolicyAlgorithm::Ed25519 => {
            let arr: [u8; 32] = public
                .try_into()
                .map_err(|_| anyhow::anyhow!("ed25519 public key must be 32 bytes"))?;
            let key = Ed25519VerifyingKey::from_bytes(&arr)?;
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            Ed25519Verifier::verify(&key, payload, &sig)
                .map_err(|_| anyhow::anyhow!("ed25519 signature mismatch"))
        }
    }
}

fn infer_signature_algorithm(record: &PolicySignature, public: &KeyMaterial) -> PolicyAlgorithm {
    if let Ok(algo) = parse_algorithm(&record.algorithm) {
        return algo;
    }
    public.algorithm
}

pub fn has_signing_key(root: Option<&Path>) -> bool {
    policy_paths(root).1.exists()
}

pub fn has_public_key(root: Option<&Path>) -> bool {
    policy_paths(root).0.exists()
}

fn read_signature_record(root: Option<&Path>) -> Result<Option<PolicySignature>> {
    let (_, _, sig_path) = policy_paths(root);
    if !sig_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&sig_path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

pub fn ensure_gitignore(root: Option<&Path>) -> Result<()> {
    let dir = repovow_dir(root);
    fs::create_dir_all(&dir)?;
    let path = dir.join(".gitignore");
    let want = format!("{POLICY_KEY_FILE}\n");
    if path.exists() {
        let existing = fs::read_to_string(&path)?;
        if existing.lines().any(|l| l.trim() == POLICY_KEY_FILE) {
            return Ok(());
        }
        let mut next = existing;
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&want);
        fs::write(&path, next)?;
    } else {
        fs::write(&path, want)?;
    }
    Ok(())
}

pub fn init_policy_named(root: Option<&Path>, algorithm: &str) -> Result<()> {
    init_policy(root, parse_algorithm(algorithm)?)
}

pub(crate) fn init_policy(root: Option<&Path>, algorithm: PolicyAlgorithm) -> Result<()> {
    let (pub_path, key_path, _) = policy_paths(root);
    fs::create_dir_all(pub_path.parent().unwrap())?;

    let (pub_hex, secret_hex) = match algorithm {
        PolicyAlgorithm::EcdsaP256 => {
            let signing = P256SigningKey::random(&mut OsRng);
            let verifying = P256VerifyingKey::from(&signing);
            let compressed = verifying.to_encoded_point(true);
            (
                to_hex(compressed.as_bytes()),
                to_hex(signing.to_bytes().as_slice()),
            )
        }
        PolicyAlgorithm::Ed25519 => {
            let signing = Ed25519SigningKey::generate(&mut OsRng);
            let verifying = signing.verifying_key();
            (
                to_hex(verifying.as_bytes()),
                to_hex(signing.to_keypair_bytes().as_slice()),
            )
        }
    };

    write_key_file(&pub_path, algorithm, &pub_hex)?;
    write_key_file(&key_path, algorithm, &secret_hex)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }
    ensure_gitignore(root)?;

    let mut config = load_config(root)?;
    config.policy.mode = PolicyMode::Required;
    save_config(&config, root)?;

    if load_state(root)?.goal.is_some() {
        sign_policy(root)?;
        let _ = crate::snapshot::write_snapshot(root);
    }

    println!(
        "Policy signing initialized ({}, {}).",
        algorithm.id(),
        algorithm.fips_note()
    );
    println!("  Public key:  {}", pub_path.display());
    println!("  Private key: {} (gitignored)", key_path.display());
    println!("  Mode: required — hooks block tools when goal is unsigned or tampered.");
    if algorithm == PolicyAlgorithm::Ed25519 {
        println!("  Note: Ed25519 is not FIPS-approved. Prefer `repovow policy init` (default ecdsa-p256) for regulated environments.");
    }
    println!("Commit policy.pub + policy.sig; keep policy.key secret (CI or lead machine only).");
    Ok(())
}

pub fn trust_pubkey(root: Option<&Path>, algorithm: Option<&str>, pubkey_hex: &str) -> Result<()> {
    let bytes = from_hex(pubkey_hex.trim())?;
    let algorithm = if let Some(name) = algorithm {
        parse_algorithm(name)?
    } else {
        match bytes.len() {
            32 => PolicyAlgorithm::Ed25519,
            33 | 65 => PolicyAlgorithm::EcdsaP256,
            n => bail!(
                "cannot infer algorithm from {n}-byte public key — pass --algorithm ecdsa-p256"
            ),
        }
    };
    validate_public_key(algorithm, &bytes)?;

    let (pub_path, _, _) = policy_paths(root);
    fs::create_dir_all(pub_path.parent().unwrap())?;
    write_key_file(&pub_path, algorithm, pubkey_hex.trim())?;

    let mut config = load_config(root)?;
    config.policy.mode = PolicyMode::Required;
    save_config(&config, root)?;

    println!(
        "Trusted policy public key saved ({}, {}).",
        algorithm.id(),
        algorithm.fips_note()
    );
    println!("  Path: {}", pub_path.display());
    println!("Mode: required — verify with `repovow policy verify` after pulling policy.sig.");
    Ok(())
}

pub fn set_mode(root: Option<&Path>, mode: PolicyMode) -> Result<()> {
    let mut config = load_config(root)?;
    config.policy.mode = mode.clone();
    save_config(&config, root)?;
    println!("Policy mode: {}", mode_label(&mode));
    Ok(())
}

pub fn parse_mode(s: &str) -> Result<PolicyMode> {
    match s.to_ascii_lowercase().as_str() {
        "off" => Ok(PolicyMode::Off),
        "warn" => Ok(PolicyMode::Warn),
        "required" => Ok(PolicyMode::Required),
        other => bail!("unknown policy mode: {other} (use off, warn, or required)"),
    }
}

fn mode_label(mode: &PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Off => "off",
        PolicyMode::Warn => "warn",
        PolicyMode::Required => "required",
    }
}

pub fn sign_policy(root: Option<&Path>) -> Result<()> {
    let state = load_state(root)?;
    let goal = state
        .goal
        .as_ref()
        .context("no active goal — set a goal before signing")?;

    let signing = read_signing_material(root).context(
        "no policy.key — run `repovow policy init` on a trusted machine or use `repovow policy trust` with team pubkey",
    )?;
    let algorithm = signing.algorithm;
    let secret = signing
        .secret_bytes
        .as_ref()
        .context("missing signing secret")?;

    let payload = canonical_payload(goal)?;
    let digest = payload_sha256(&payload);
    let signature = sign_bytes(algorithm, secret, &payload)?;

    let record = PolicySignature {
        v: 1,
        algorithm: algorithm.id().into(),
        signature: to_hex(&signature),
        signed_at: utcnow(),
        payload_sha256: digest.clone(),
    };

    let (_, _, sig_path) = policy_paths(root);
    write_json_atomic(&sig_path, &serde_json::to_value(&record)?)?;
    write_policy_md(root)?;
    log_event(
        root,
        "policy_signed",
        serde_json::json!({"payload_sha256": digest, "algorithm": algorithm.id()}),
    )?;
    Ok(())
}

pub fn verify_policy(root: Option<&Path>) -> Result<PolicyStatus> {
    let config = load_config(root)?;
    if config.policy.mode == PolicyMode::Off {
        return Ok(PolicyStatus::Off);
    }

    let state = load_state(root)?;
    let Some(goal) = state.goal.as_ref() else {
        return Ok(PolicyStatus::NoGoal);
    };

    if !has_public_key(root) {
        return Ok(PolicyStatus::MissingPublicKey);
    }

    let Some(record) = read_signature_record(root)? else {
        return Ok(PolicyStatus::Unsigned);
    };

    let payload = canonical_payload(goal)?;
    let digest = payload_sha256(&payload);
    if digest != record.payload_sha256 {
        return Ok(PolicyStatus::Tampered);
    }

    let public = read_public_material(root)?;
    let algorithm = infer_signature_algorithm(&record, &public);
    let sig_bytes = from_hex(&record.signature)?;

    if verify_bytes(algorithm, &public.public_bytes, &payload, &sig_bytes).is_err() {
        return Ok(PolicyStatus::InvalidSignature);
    }

    Ok(PolicyStatus::Valid {
        signed_at: record.signed_at,
        algorithm: algorithm.id().into(),
    })
}

pub fn hook_block_reason(root: Option<&Path>) -> Result<Option<String>> {
    let config = load_config(root)?;
    match config.policy.mode {
        PolicyMode::Off => Ok(None),
        PolicyMode::Warn => {
            let status = verify_policy(root)?;
            if !status.is_ok()
                && status != PolicyStatus::NoGoal
                && claim_policy_warning(root, &status)?
            {
                eprintln!(
                    "RepoVow policy warning: {} — {}",
                    status.label(),
                    status.detail()
                );
            }
            Ok(None)
        }
        PolicyMode::Required => {
            let status = verify_policy(root)?;
            if status.is_ok() {
                return Ok(None);
            }
            Ok(Some(format!(
                "RepoVow policy enforcement ({}) — {}",
                status.label(),
                status.detail()
            )))
        }
    }
}

fn claim_policy_warning(root: Option<&Path>, status: &PolicyStatus) -> Result<bool> {
    let path = repovow_dir(root).join(POLICY_WARNING_MARKER_FILE);
    let session = load_state(root)?.sessions;
    let marker = read_json(&path, serde_json::json!({}))?;
    if marker["session"].as_u64() == Some(session as u64)
        && marker["status"].as_str() == Some(status.label())
    {
        return Ok(false);
    }
    write_json_atomic(
        &path,
        &serde_json::json!({"session": session, "status": status.label()}),
    )?;
    Ok(true)
}

pub fn after_goal_change(root: Option<&Path>) -> Result<()> {
    if !has_signing_key(root) {
        return Ok(());
    }
    sign_policy(root)
}

pub fn protect_goal_after_pull(
    root: Option<&Path>,
    local_before: &crate::state::RepoVowState,
) -> Result<()> {
    let config = load_config(root)?;
    if config.policy.mode != PolicyMode::Required {
        return Ok(());
    }

    let status = verify_policy(root)?;
    if status.is_ok() {
        return Ok(());
    }

    let mut state = load_state(root)?;
    let pulled_goal = state.goal.clone();
    state.goal = local_before.goal.clone();
    crate::state::save_state(&mut state, root)?;
    log_event(
        root,
        "policy_pull_blocked",
        serde_json::json!({
            "status": status.label(),
            "rejected_goal": pulled_goal.map(|g| g.title),
        }),
    )?;
    eprintln!(
        "RepoVow policy: cloud pull goal rejected ({}) — kept local signed goal.",
        status.label()
    );
    Ok(())
}

pub fn write_policy_md(root: Option<&Path>) -> Result<std::path::PathBuf> {
    let path = repovow_dir(root).join(POLICY_MD_FILE);
    let state = load_state(root)?;
    let status = verify_policy(root).unwrap_or(PolicyStatus::Unsigned);
    let algo_line = has_public_key(root)
        .then(|| read_public_material(root).ok())
        .flatten()
        .map(|k| {
            format!(
                "Algorithm: **{}** ({})",
                k.algorithm.id(),
                k.algorithm.fips_note()
            )
        })
        .unwrap_or_else(|| {
            format!("Algorithm: **{DEFAULT_POLICY_ALGORITHM}** (default for new installs)")
        });

    let mut lines = vec![
        "# RepoVow policy (signed goal)".into(),
        String::new(),
        "_Cryptographic policy for title, acceptance, and constraints only. \
         Agent-written progress and failures live in `snapshot.md` and are **not** signed._"
            .into(),
        String::new(),
        algo_line,
        String::new(),
    ];

    let badge = match &status {
        PolicyStatus::Off => "Policy signing: **off**".into(),
        PolicyStatus::Valid {
            signed_at,
            algorithm,
        } => {
            format!("Policy signature: **valid** ({algorithm}, signed {signed_at})")
        }
        other => format!("Policy signature: **{}**", other.label().to_uppercase()),
    };
    lines.push(badge);
    lines.push(String::new());

    if let Some(goal) = &state.goal {
        lines.push("## Goal".into());
        lines.push(format!("**{}**", goal.title));
        lines.push(String::new());
        if !goal.acceptance.is_empty() {
            lines.push("### Acceptance".into());
            for item in &goal.acceptance {
                lines.push(format!("- {item}"));
            }
            lines.push(String::new());
        }
        if !goal.constraints.is_empty() {
            lines.push("### Constraints".into());
            for item in &goal.constraints {
                lines.push(format!("- {item}"));
            }
            lines.push(String::new());
        }
    } else {
        lines.push("_No active goal._".into());
        lines.push(String::new());
    }

    lines.push(format!(
        "_Verify: `repovow policy verify` · Re-sign: `repovow policy sign` · Mode: {}_",
        mode_label(&load_config(root)?.policy.mode)
    ));

    let text = lines.join("\n") + "\n";
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, text)?;
    Ok(path)
}

/// JSON for RepoVow Cloud dashboard (`policy.label`, `policy.ok`, etc.).
pub fn policy_status_json(root: Option<&Path>) -> serde_json::Value {
    let config = load_config(root).unwrap_or_default();
    let status = verify_policy(root).unwrap_or(PolicyStatus::Off);
    let (ok, detail) = doctor_detail(root);
    serde_json::json!({
        "mode": mode_label(&config.policy.mode),
        "label": status.label(),
        "ok": ok,
        "detail": detail,
    })
}

pub fn doctor_detail(root: Option<&Path>) -> (bool, String) {
    let config = match load_config(root) {
        Ok(c) => c,
        Err(e) => return (false, e.to_string()),
    };
    match config.policy.mode {
        PolicyMode::Off => (
            true,
            "off — `repovow policy init` to enable signed goals".into(),
        ),
        PolicyMode::Warn | PolicyMode::Required => {
            let status = verify_policy(root).unwrap_or(PolicyStatus::Unsigned);
            let ok = config.policy.mode == PolicyMode::Warn || status.is_ok();
            let mode = mode_label(&config.policy.mode);
            let algo = has_public_key(root)
                .then(|| read_public_material(root).ok())
                .flatten()
                .map(|k| k.algorithm.id().to_string())
                .unwrap_or_else(|| "unknown".into());
            (
                ok,
                format!("{mode} — {algo} — {} ({})", status.label(), status.detail()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{save_config, save_state, Goal, RepoVowConfig, RepoVowState};

    fn fixture_goal() -> Goal {
        Goal {
            title: "Ship v0.4".into(),
            acceptance: vec!["tests pass".into()],
            constraints: vec!["no secrets in repo".into()],
            started_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn setup(root: &Path) {
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        save_config(&RepoVowConfig::default(), Some(root)).unwrap();
        let mut state = RepoVowState {
            goal: Some(fixture_goal()),
            ..RepoVowState::default()
        };
        save_state(&mut state, Some(root)).unwrap();
    }

    #[test]
    fn p256_sign_and_verify_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup(root);
        init_policy(Some(root), PolicyAlgorithm::EcdsaP256).unwrap();
        let status = verify_policy(Some(root)).unwrap();
        assert!(matches!(status, PolicyStatus::Valid { .. }));

        let mut state = load_state(Some(root)).unwrap();
        state.goal.as_mut().unwrap().title = "Tampered".into();
        save_state(&mut state, Some(root)).unwrap();
        assert_eq!(verify_policy(Some(root)).unwrap(), PolicyStatus::Tampered);
    }

    #[test]
    fn ed25519_legacy_sign_and_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup(root);
        init_policy(Some(root), PolicyAlgorithm::Ed25519).unwrap();
        assert!(matches!(
            verify_policy(Some(root)).unwrap(),
            PolicyStatus::Valid { .. }
        ));
    }

    #[test]
    fn trust_pubkey_verifies_team_sig() {
        let lead = tempfile::tempdir().unwrap();
        let dev = tempfile::tempdir().unwrap();
        setup(lead.path());
        setup(dev.path());

        init_policy(Some(lead.path()), PolicyAlgorithm::EcdsaP256).unwrap();
        sign_policy(Some(lead.path())).unwrap();

        let pub_content =
            std::fs::read_to_string(lead.path().join(crate::REPOVOW_DIR).join(POLICY_PUB_FILE))
                .unwrap();
        let sig_raw =
            std::fs::read_to_string(lead.path().join(crate::REPOVOW_DIR).join(POLICY_SIG_FILE))
                .unwrap();
        std::fs::write(
            dev.path().join(crate::REPOVOW_DIR).join(POLICY_SIG_FILE),
            sig_raw,
        )
        .unwrap();

        let pub_hex = pub_content.lines().nth(1).expect("two-line policy.pub");
        trust_pubkey(Some(dev.path()), Some("ecdsa-p256"), pub_hex).unwrap();
        assert!(matches!(
            verify_policy(Some(dev.path())).unwrap(),
            PolicyStatus::Valid { .. }
        ));
    }

    #[test]
    fn policy_warning_is_claimed_once_per_session_and_status() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup(root);
        let status = PolicyStatus::MissingPublicKey;

        assert!(claim_policy_warning(Some(root), &status).unwrap());
        assert!(!claim_policy_warning(Some(root), &status).unwrap());
        assert!(claim_policy_warning(Some(root), &PolicyStatus::Unsigned).unwrap());

        let mut state = load_state(Some(root)).unwrap();
        state.sessions += 1;
        save_state(&mut state, Some(root)).unwrap();
        assert!(claim_policy_warning(Some(root), &status).unwrap());
    }
}
