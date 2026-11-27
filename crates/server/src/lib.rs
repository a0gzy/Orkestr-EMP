pub mod api;
mod automation;
pub mod deployment;
mod inventory;
mod library;
pub mod platform;
mod probe;
pub mod runtime;
mod scheduling;
#[cfg(test)]
mod tests_pipeline;
mod workflow;

use chrono::Utc;
use orkestr_protocol::*;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
pub fn new_token(label: String, role: Role, days: u16) -> Result<IssuedToken, String> {
    if label.trim().is_empty()
        || label.len() > 80
        || label.chars().any(char::is_control)
        || !(1..=365).contains(&days)
    {
        return Err("INVALID_TOKEN_POLICY".into());
    }
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| "RANDOM_UNAVAILABLE")?;
    let mut token = Zeroizing::new(String::with_capacity(64));
    use std::fmt::Write;
    for b in bytes.iter() {
        write!(&mut *token, "{b:02x}").map_err(|_| "RANDOM_UNAVAILABLE")?;
    }
    Ok(IssuedToken {
        actor: Actor {
            id: Uuid::new_v4(),
            label,
            role,
            expires_at: Utc::now() + chrono::Duration::days(days.into()),
        },
        token,
    })
}
