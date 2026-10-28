//! pptx → pdf conversion via the local PowerPoint COM automation (PRD §4.2 Step2:
//! Windows 走 PowerPoint COM 直出；失败则提示"请先另存为 PDF"). Same process
//! discipline as cc-core's SapiEngine: spawned as an argument array with an
//! UTF-16LE EncodedCommand, paths embedded as single-quoted literals — never a
//! shell string. A hung conversion (modal dialog, dead Office) is killed after a
//! timeout instead of freezing the UI thread.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// PRD F2 budgets COM conversion at ≤ 20 s for a typical deck; the 60 s ceiling is
/// the safety net for large decks and slow machines, not a goal.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(60);

/// Convert `pptx` into `pdf_out` through PowerPoint's own SaveAs (format 32 = PDF).
/// Errors are plain messages: the command layer wraps them into the
/// "请先另存为 PDF" prompt for the UI.
pub(super) fn convert_pptx_to_pdf(pptx: &Path, pdf_out: &Path) -> std::result::Result<(), String> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $pp = New-Object -ComObject PowerPoint.Application\n\
         try {{\n\
           $pres = $pp.Presentations.Open({pptx}, $true, $false, $false)\n\
           try {{ $pres.SaveAs({pdf}, 32) }} finally {{ $pres.Close() }}\n\
         }} finally {{ $pp.Quit() }}\n",
        pptx = ps_quote(&pptx.display().to_string()),
        pdf = ps_quote(&pdf_out.display().to_string()),
    );
    let mut child = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded_command(&script),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 PowerShell：{e}"))?;

    let deadline = Instant::now() + CONVERT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => return Err(format!("等待 PowerShell 失败：{e}")),
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!(
                "转换超时（{} 秒）已终止",
                CONVERT_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("读取转换输出失败：{e}"))?;
    if output.status.success() && pdf_out.is_file() {
        return Ok(());
    }
    let tail: String = String::from_utf8_lossy(&output.stderr)
        .lines()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Err(if tail.trim().is_empty() {
        "PowerPoint 转换未产出 PDF 文件".to_string()
    } else {
        tail
    })
}

/// PowerShell single-quoted literal: the only escape is `'` → `''` (same discipline
/// as cc-core's SapiEngine — no text content can break out or execute).
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `-EncodedCommand` payload: the whole script as UTF-16LE base64 (no codepage
/// issues with Chinese paths, no way to smuggle a second argument).
fn encoded_command(script: &str) -> String {
    let mut utf16 = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    BASE64.encode(utf16)
}
