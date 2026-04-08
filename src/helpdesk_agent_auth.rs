use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const HELPDESK_AGENT_TOKEN_HEADER: &str = "x-helpdesk-agent-token";

pub fn generate_helpdesk_agent_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_helpdesk_agent_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.trim().as_bytes());
    hex::encode(hasher.finalize())
}

pub fn helpdesk_agent_token_hint(raw_token: &str) -> Option<String> {
    let trimmed = raw_token.trim();
    if trimmed.is_empty() {
        return None;
    }

    let suffix_len = trimmed.len().min(6);
    Some(format!("...{}", &trimmed[trimmed.len() - suffix_len..]))
}
