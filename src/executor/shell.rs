// src/executor/shell.rs
//! قرارداد Placeholder:  {{shell_output ["arg1", "arg2", ...]}}
//!
//! - **داخل** یک مقدار رشته‌ای JSON → خروجی JSON-escaped در همان رشته
//!   درج می‌شود (مثل قبل).
//! - **بیرون** از هر رشته (به‌عنوان یه مقدار JSON خالی) → خروجی **بدون
//!   escape** (بعد از trim فضای سفید) درج می‌شود. اسکریپت باید خودش JSON
//!   معتبر (object/array/number/bool/null) چاپ کند.
//!
//! - فقط اولین Placeholder اجرا می‌شود؛ اگر بیش از یکی باشد، هیچ‌کدام.
//! - مسیر اسکریپت هرگز از پاسخ مدل خوانده نمی‌شود.

use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command;

const PLACEHOLDER_START: &str = "{{shell_output";
const PLACEHOLDER_END: &str = "}}";

pub const SHELL_INSTRUCTION: &str = r#"
### SHELL EXECUTION (available)

You have ONE shell execution available per attempt. To request it, write exactly
one placeholder of this form:

  {{shell_output ["arg1", "arg2", ...]}}

The content inside MUST be a valid JSON array of strings. These strings are
passed as argv directly to a fixed script on the host. They are NOT interpreted
by a shell — no metacharacters, no globbing.

WHERE the placeholder goes determines how stdout is spliced in:

1. INSIDE a JSON string value:
     {"body":"{{shell_output [\"id\"]}}"}
   stdout is JSON-escaped (newlines → \n, quotes → \", etc.) and inserted
   into the string. Use when the placeholder supplies a scalar string.

2. OUTSIDE any JSON string, as a bare JSON value:
     {"requests": {{shell_output ["id"]}}}
   stdout is inserted VERBATIM (trailing whitespace trimmed). The script MUST
   therefore print valid JSON (object, array, number, true/false, null). Use
   when the placeholder supplies an entire JSON structure.

Rules:
- Emit AT MOST ONE placeholder per response. More than one → NONE execute.
- Response must remain valid JSON after substitution.
- Script exits 0 for its output to be used.
"#;

#[derive(Debug)]
pub enum ShellError {
    InvalidArgs(String),
    MultiplePlaceholders(usize),
    SpawnFailed(String),
    Timeout,
    NonZeroExit { code: Option<i32>, stderr: String },
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::InvalidArgs(m) => write!(f, "invalid shell args: {}", m),
            ShellError::MultiplePlaceholders(n) => {
                write!(f, "{} placeholders found; only ONE allowed per attempt", n)
            }
            ShellError::SpawnFailed(m) => write!(f, "spawn failed: {}", m),
            ShellError::Timeout => write!(f, "shell execution timed out"),
            ShellError::NonZeroExit { code, stderr } => write!(
                f,
                "non-zero exit {:?}: {}",
                code,
                stderr.chars().take(200).collect::<String>()
            ),
        }
    }
}

pub struct ShellResult {
    pub stdout: String,
    #[allow(dead_code)]
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub struct ShellCallSummary {
    pub args: Vec<String>,
    pub result: Result<ShellResult, ShellError>,
}

pub struct ProcessedResponse {
    pub modified: String,
    pub shell_call: Option<ShellCallSummary>,
}

pub struct ShellRuntime {
    script_path: PathBuf,
    timeout: Duration,
}

struct PlaceholderSpan {
    start: usize,
    end: usize,
    args_text: String,
}

impl ShellRuntime {
    /// ساخت runtime. مسیر اسکریپت را اعتبارسنجی می‌کند: باید وجود داشته
    /// باشد و (در Unix) بیت اجراپذیر داشته باشد. در غیر این صورت Lab
    /// قبل از هر اجرایی fail می‌شود (نه panic).
    pub fn new(script_path: impl Into<PathBuf>, timeout_secs: u64) -> Result<Self, String> {
        let mut path = script_path.into();
        if !path.is_absolute() {
            if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
                path = PathBuf::from(manifest).join(&path);
            }
        }
        if !path.exists() {
            return Err(format!("shell script not found: {}", path.display()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&path)
                .map_err(|e| format!("cannot stat shell script '{}': {}", path.display(), e))?;
            if meta.permissions().mode() & 0o111 == 0 {
                return Err(format!(
                    "shell script is not executable: {}",
                    path.display()
                ));
            }
        }
        Ok(Self {
            script_path: path,
            timeout: Duration::from_secs(timeout_secs.max(1)),
        })
    }

    /// پردازش پاسخ خام مدل: صفر → بدون تغییر. یک Placeholder معتبر →
    /// اجرا + جایگزینی JSON-escaped. بیش از یکی یا args نامعتبر → خطا،
    /// بدون تغییر در متن.
    pub async fn process_response(&self, raw: &str) -> ProcessedResponse {
        let spans = find_all_placeholders(raw);
        match spans.len() {
            0 => ProcessedResponse {
                modified: raw.to_string(),
                shell_call: None,
            },
            1 => {
                let span = &spans[0];
                let args = match parse_placeholder_args(&span.args_text) {
                    Ok(a) => a,
                    Err(e) => {
                        return ProcessedResponse {
                            modified: raw.to_string(),
                            shell_call: Some(ShellCallSummary {
                                args: vec![],
                                result: Err(e),
                            }),
                        };
                    }
                };
                let result = self.execute(&args).await;
                let inside = is_inside_json_string(raw, span.start);
                let replacement = match &result {
                    Ok(r) => {
                        if inside {
                            json_escape(&r.stdout) // مثل قبل
                        } else {
                            r.stdout.trim().to_string() // RAW — اسکریپت خودش JSON داده
                        }
                    }
                    Err(_) => String::new(),
                };
                let modified = format!("{}{}{}", &raw[..span.start], replacement, &raw[span.end..]);
                ProcessedResponse {
                    modified,
                    shell_call: Some(ShellCallSummary { args, result }),
                }
            }
            n => ProcessedResponse {
                modified: raw.to_string(),
                shell_call: Some(ShellCallSummary {
                    args: vec![],
                    result: Err(ShellError::MultiplePlaceholders(n)),
                }),
            },
        }
    }

    async fn execute(&self, args: &[String]) -> Result<ShellResult, ShellError> {
        let mut cmd = Command::new(&self.script_path);
        cmd.args(args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        {
            // گروه فرایند اختصاصی: pgid == pid فرزند. هر فرزندِ فرزند هم
            // در همین گروه قرار می‌گیرد، پس با SIGKILL به -pid همه می‌میرند.
            cmd.process_group(0);
        }

        let child = cmd
            .spawn()
            .map_err(|e| ShellError::SpawnFailed(e.to_string()))?;
        let pid = child.id();

        match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let exit_code = output.status.code();
                if output.status.success() {
                    Ok(ShellResult {
                        stdout,
                        stderr,
                        exit_code,
                    })
                } else {
                    Err(ShellError::NonZeroExit {
                        code: exit_code,
                        stderr,
                    })
                }
            }
            Ok(Err(e)) => Err(ShellError::SpawnFailed(e.to_string())),
            Err(_elapsed) => {
                // Timeout: ابتدا گروه فرایند را بکش (شامل فرزندانِ فرزند).
                // child خودش با kill_on_drop drop خواهد شد، ولی درختِ
                // زیرمجموعه با SIGKILL به -pgid پاک‌سازی می‌شود.
                #[cfg(unix)]
                if let Some(pid) = pid {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                Err(ShellError::Timeout)
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Placeholder parsing
// ═══════════════════════════════════════════════════════════════════════

fn find_all_placeholders(text: &str) -> Vec<PlaceholderSpan> {
    let mut spans = Vec::new();
    let mut search_from = 0;
    while let Some(rel_start) = text[search_from..].find(PLACEHOLDER_START) {
        let start = search_from + rel_start;
        let after = start + PLACEHOLDER_START.len();
        if let Some(rel_close) = text[after..].find(PLACEHOLDER_END) {
            let close_abs = after + rel_close;
            spans.push(PlaceholderSpan {
                start,
                end: close_abs + PLACEHOLDER_END.len(),
                args_text: text[after..close_abs].trim().to_string(),
            });
            search_from = close_abs + PLACEHOLDER_END.len();
        } else {
            break;
        }
    }
    spans
}

/// یک لایه escape را برمی‌دارد. از String::replace استفاده نمی‌کند چون
/// ترتیب replaceها می‌تواند اشتباه باشد؛ به‌جایش char-by-char کار می‌کند.
/// هر `\X` به `X` تبدیل می‌شود (به‌جز escapeهای شناخته‌شده که معادل
/// کاراکتر مربوطه را می‌دهند).
fn unescape_one_level(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars();
    while let Some(c) = iter.next() {
        if c == '\\' {
            match iter.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{0008}'),
                Some('f') => out.push('\u{000C}'),
                Some('u') => {
                    // \uXXXX — چهار رقم هگز
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(h) = iter.next() {
                            hex.push(h);
                        } else {
                            break;
                        }
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                            continue;
                        }
                    }
                    // اگر parse نشد، عیناً بگذار
                    out.push('\\');
                    out.push('u');
                    out.push_str(&hex);
                }
                Some(other) => {
                    // escape ناشناخته — عیناً نگه دار
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// مدل ممکن است placeholder را در چند لایه‌ی JSON قرار دهد (بسته به
/// عمق فیلدهای تودرتوی payload). تا ۴ بار unescape می‌کنیم و اولین
/// تلاش موفقی که `Vec<String>` را parse کند برمی‌گردانیم.
fn parse_placeholder_args(args_text: &str) -> Result<Vec<String>, ShellError> {
    let mut current = args_text.trim().to_string();
    for level in 0..=4 {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(&current) {
            eprintln!(
                "[SHELL] parsed args after {} unescape level(s): {:?}",
                level, v
            );
            return Ok(v);
        }
        current = unescape_one_level(&current);
    }
    Err(ShellError::InvalidArgs(format!(
        "failed to parse args after 4 unescape levels (raw: {:?})",
        args_text
    )))
}

/// JSON-escape یک رشته بدون کوتیشن‌های بیرونی. از serde_json استفاده می‌کند
/// تا تمام escape ها (", \\, \n, \r, \t, \b, \f, کنترل‌کاراکترها < 0x20،
/// و Unicode control chars) دقیقاً مطابق RFC 8259 انجام شوند.
pub fn json_escape(s: &str) -> String {
    let quoted = match serde_json::to_string(s) {
        Ok(q) => q,
        Err(_) => return String::new(),
    };
    if quoted.len() >= 2 && quoted.starts_with('"') && quoted.ends_with('"') {
        quoted[1..quoted.len() - 1].to_string()
    } else {
        quoted
    }
}

/// آیا موقعیت `pos` داخل یک رشته‌ی JSON است؟
/// با شمارش کوتیشن‌های escape‌نشده تا `pos` (فرد = داخل، زوج = خارج).
fn is_inside_json_string(text: &str, pos: usize) -> bool {
    let mut depth: u32 = 0;
    let mut escaped = false;
    for c in text[..pos].chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            depth += 1;
        }
    }
    depth % 2 == 1
}
// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════
#[tokio::test]
#[cfg(unix)]
async fn bare_placeholder_inserts_raw_stdout() {
    use std::os::unix::fs::PermissionsExt;
    let script = std::env::temp_dir().join(format!(
        "aegis_bare_ph_{}.sh", std::process::id()
    ));
    std::fs::write(&script, "#!/bin/sh\nprintf '%s' '[{\"a\":1},{\"b\":2}]'\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let rt = ShellRuntime::new(&script, 5).unwrap();
    let raw = r#"{"requests": {{shell_output ["ignored"]}}}"#;
    let out = rt.process_response(raw).await;
    assert!(out.shell_call.unwrap().result.is_ok());
    assert_eq!(out.modified, r#"{"requests": [{"a":1},{"b":2}]}"#);
    let _: serde_json::Value = serde_json::from_str(&out.modified).unwrap();
    let _ = std::fs::remove_file(&script);
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_basic_quotes() {
        assert_eq!(json_escape(r#"a"b"#), r#"a\"b"#);
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(json_escape(r"a\b"), r"a\\b");
    }

    #[test]
    fn escape_newline() {
        assert_eq!(json_escape("a\nb"), r"a\nb");
    }

    #[test]
    fn escape_control_chars() {
        // Tab, carriage return, and a raw control byte.
        let s = format!("a\tb\rc{}d", 0x07 as char);
        let escaped = json_escape(&s);
        // Round-trip: wrapping in quotes and parsing back must yield original.
        let back: String = serde_json::from_str(&format!("\"{}\"", escaped)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn escape_roundtrip_complex() {
        let s = "uid=0(root)\ngid=0 \"quoted\" \\slash\\ \ttab";
        let escaped = json_escape(s);
        let back: String = serde_json::from_str(&format!("\"{}\"", escaped)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn find_zero() {
        assert!(find_all_placeholders("no placeholder here").is_empty());
    }

    #[test]
    fn find_one() {
        let spans = find_all_placeholders(r#"before {{shell_output ["id"]}} after"#);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].args_text, r#"["id"]"#);
    }

    #[test]
    fn find_multiple() {
        let text = r#"{{shell_output ["a"]}} and {{shell_output ["b"]}}"#;
        assert_eq!(find_all_placeholders(text).len(), 2);
    }

    #[test]
    fn parse_plain() {
        assert_eq!(
            parse_placeholder_args(r#"["id", "user 1"]"#).unwrap(),
            vec!["id", "user 1"]
        );
    }

    #[test]
    fn parse_escaped() {
        assert_eq!(
            parse_placeholder_args(r#"[\"id\", \"user 1\"]"#).unwrap(),
            vec!["id", "user 1"]
        );
    }

    #[test]
    fn parse_invalid() {
        assert!(parse_placeholder_args("not an array").is_err());
    }

    #[test]
    fn parse_double_escaped_quotes() {
        // مدل در دو لایه JSON escape می‌کند (سناریوی real-world).
        assert_eq!(
            parse_placeholder_args(r#"[\\\"id\\\"]"#).unwrap(),
            vec!["id"]
        );
    }

    #[test]
    fn parse_triple_escaped_quotes() {
        assert_eq!(
            parse_placeholder_args(r#"[\\\\\\\"id\\\\\\\"]"#).unwrap(),
            vec!["id"]
        );
    }

    #[test]
    fn parse_zero_escapes_still_works() {
        assert_eq!(parse_placeholder_args(r#"["id"]"#).unwrap(), vec!["id"]);
    }

    #[test]
    fn parse_single_escape_still_works() {
        assert_eq!(parse_placeholder_args(r#"[\"id\"]"#).unwrap(), vec!["id"]);
    }

    #[tokio::test]
    async fn process_no_placeholder_passthrough() {
        let rt = ShellRuntime::new("/bin/true", 5).unwrap_or_else(|_| {
            // fallback if /bin/true missing (very unlikely on Unix).
            panic!("/bin/true missing — cannot test")
        });
        let out = rt.process_response("nothing here").await;
        assert_eq!(out.modified, "nothing here");
        assert!(out.shell_call.is_none());
    }

    #[tokio::test]
    async fn process_multiple_placeholders_no_exec() {
        let rt = ShellRuntime::new("/bin/true", 5).unwrap();
        let text = r#"{{shell_output ["a"]}} {{shell_output ["b"]}}"#;
        let out = rt.process_response(text).await;
        assert_eq!(out.modified, text); // unchanged
        let call = out.shell_call.unwrap();
        assert!(matches!(
            call.result,
            Err(ShellError::MultiplePlaceholders(2))
        ));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn execute_success() {
        let rt = ShellRuntime::new("/bin/echo", 5).unwrap();
        let result = rt.execute(&["hello".to_string()]).await.unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.exit_code, Some(0));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn process_executes_and_substitutes() {
        let rt = ShellRuntime::new("/bin/echo", 5).unwrap();
        let raw = r#"{"body":"{{shell_output [\"hello\"]}}"}"#;
        let out = rt.process_response(raw).await;
        assert!(out.shell_call.unwrap().result.is_ok());
        assert_eq!(out.modified, r#"{"body":"hello\n"}"#);
        // Verify the result is still valid JSON.
        let _: serde_json::Value = serde_json::from_str(&out.modified).unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn timeout_kills_child_and_grandchild() {
        use std::os::unix::fs::PermissionsExt;

        let marker =
            std::env::temp_dir().join(format!("aegis_shell_marker_{}.txt", std::process::id()));
        let script =
            std::env::temp_dir().join(format!("aegis_shell_script_{}.sh", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        std::fs::write(
            &script,
            format!("#!/bin/bash\nsleep 3\necho done > {}\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let rt = ShellRuntime::new(&script, 1).unwrap();
        let start = std::time::Instant::now();
        let result = rt.execute(&[]).await;
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(ShellError::Timeout)),
            "expected Timeout, got {:?}",
            result.err()
        );
        assert!(
            elapsed.as_secs() < 2,
            "timeout should fire ~1s, took {:?}",
            elapsed
        );

        // Wait past the sleep duration: if sleep survived, marker exists.
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        assert!(
            !marker.exists(),
            "child 'sleep 3' survived timeout — process group kill failed"
        );

        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&marker);
    }
}
