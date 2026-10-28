//! 《声音授权确认》(声纹克隆合规): cloning a voice requires an explicit user
//! acknowledgement before synthesis. File-as-truth in the project directory so every
//! shell (UI / CLI / MCP) reads and writes the same record.

use crate::error::{Error, Result};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The compliance acknowledgement file (PRD §4.2 Step3: 必须勾选《声音授权确认》).
pub const VOICE_CONSENT_FILE: &str = "voice_consent.txt";

/// True when the acknowledgement was recorded for this project.
pub fn voice_consent_granted(project_dir: &Path) -> bool {
    std::fs::read_to_string(project_dir.join(VOICE_CONSENT_FILE))
        .map(|text| text.lines().next().is_some_and(|l| l.trim() == "granted"))
        .unwrap_or(false)
}

/// Gate a voice-clone run (MiMo engine): every shell must call this before synthesis.
pub fn require_voice_clone_consent(project_dir: &Path) -> Result<()> {
    if voice_consent_granted(project_dir) {
        return Ok(());
    }
    Err(Error::InvalidArgument(
        "语音克隆需要先勾选《声音授权确认》（本人声音或已获授权）：UI 在「配置声音」一步勾选，CLI 运行 `voice_consent --grant`".into(),
    ))
}

/// What the user chose for the voice-clone acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    Grant,
    Revoke,
}

impl From<bool> for ConsentDecision {
    fn from(granted: bool) -> Self {
        if granted {
            ConsentDecision::Grant
        } else {
            ConsentDecision::Revoke
        }
    }
}

/// Record or revoke the acknowledgement (atomic write; revocation removes the file).
pub fn set_voice_consent(project_dir: &Path, decision: ConsentDecision) -> Result<()> {
    let path = project_dir.join(VOICE_CONSENT_FILE);
    if decision == ConsentDecision::Grant {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // epoch seconds are enough precision for an audit trail; keep the file simple
        let tmp = project_dir.join("voice_consent.txt.tmp");
        std::fs::write(&tmp, format!("granted\n{ts}\n"))?;
        std::fs::rename(&tmp, &path)?;
    } else {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}
