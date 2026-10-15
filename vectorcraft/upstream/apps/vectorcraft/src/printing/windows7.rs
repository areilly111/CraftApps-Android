//! The legacy viewer print verb is needed on Windows 7, where Windows.Data.Pdf is absent.
use std::path::PathBuf;
use std::process::Command;
use vectorcraft_ui_egui::print::{PrintJob, Printer};

/// Where a job's PDF waits for the spooler: a fresh file in the temp folder. Files of jobs more
/// than an hour old (the print verb reads them after Print returns) are removed first.
fn job_file(pdf: &[u8]) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("vectorcraft-print");
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600);
        if old {
            // Another job may still be reading it: leave it for next time.
            let _ = std::fs::remove_file(e.path());
        }
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let path = dir.join(format!("job-{}-{stamp}.pdf", std::process::id()));
    // The viewer reads asynchronously, so this cannot use immediate drop cleanup. Never
    // truncate an existing path, and remove incomplete files or refused launches immediately.
    use std::io::Write as _;
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("can't create {}: {e}", path.display()))?;
    if let Err(e) = file.write_all(pdf).and_then(|_| file.flush()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(format!("can't write {}: {e}", path.display()));
    }
    Ok(path)
}

/// Run `c` → its output, or its error message.
fn output(c: &mut Command) -> Result<String, String> {
    let program = c.get_program().to_string_lossy().to_string();
    let out = c.output().map_err(|e| format!("can't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("{program} failed ({})", out.status) } else { err })
}

/// PowerShell's `<Default>\t<Name>` lines → the printers.
fn windows_printers(text: &str) -> Vec<Printer> {
    text.lines()
        .filter_map(|l| l.trim_end_matches('\r').split_once('\t'))
        .filter(|(_, name)| !name.trim().is_empty())
        .map(|(default, name)| Printer { name: name.trim().to_string(), default: default.trim().eq_ignore_ascii_case("true") })
        .collect()
}

/// `s` as a PowerShell single-quoted string.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn powershell(script: &str) -> Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = Command::new(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
    );
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!("$ErrorActionPreference = 'Stop'; [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false); {script}"),
    ])
    .creation_flags(CREATE_NO_WINDOW);
    c
}

pub fn printers() -> Vec<Printer> {
    let script = "Get-WmiObject Win32_Printer | ForEach-Object { \"$($_.Default)`t$($_.Name)\" }";
    output(&mut powershell(script)).map(|t| windows_printers(&t)).unwrap_or_default()
}

fn submit(job: &PrintJob, file: &std::path::Path) -> Result<String, String> {
    // The PDF viewer's print verb reads the file after this returns: it stays for a while.
    let path = ps_quote(&file.to_string_lossy());
    let script = match job.printer {
        Some(p) => format!(
            "Start-Process -FilePath {path} -Verb PrintTo -ArgumentList {} -WindowStyle Hidden",
            ps_quote(&format!("\"{}\"", p.replace('"', "")))
        ),
        None => format!("Start-Process -FilePath {path} -Verb Print -WindowStyle Hidden"),
    };
    output(&mut powershell(&script)).map(|_| String::new()).map_err(|e| format!("no app prints PDF files here ({e})"))
}

pub fn setup(_printer: Option<&str>) -> Result<(), String> {
    let mut command = {
        let mut command = Command::new("control.exe");
        command.arg("printers");
        command
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn print(job: &PrintJob) -> Result<String, String> {
    let file = job_file(job.pdf)?;
    let result = submit(job, &file);
    if result.is_err() {
        // No viewer accepted the launch, so no asynchronous reader needs this document.
        let _ = std::fs::remove_file(file);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn powershell_lines_and_quotes_are_preserved() {
        assert_eq!(windows_printers("True\tOffice Laser\r\nFalse\t "), [Printer { name: "Office Laser".into(), default: true }]);
        assert_eq!(ps_quote("it's.pdf"), "'it''s.pdf'");
    }
    #[test]
    fn job_files_hold_the_pdf_for_the_async_viewer() {
        let path = job_file(b"%PDF-1.7 test").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7 test");
        std::fs::remove_file(path).unwrap();
    }
}
