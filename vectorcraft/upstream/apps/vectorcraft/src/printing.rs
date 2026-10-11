//! File → Print on the desktop ([`SystemPrint`]): the system's printers and print queue. macOS and
//! Linux go through CUPS (`lpstat` lists the printers, `lp` queues a temporary copy of the job's
//! PDF); Windows asks PowerShell for the printers and prints through the shell's print verb of
//! the PDF viewer.

use std::path::PathBuf;
use std::process::Command;

use vectorcraft_ui_egui::print::{PrintJob, PrintService, Printer};

/// The system's printing.
pub struct SystemPrint;

impl PrintService for SystemPrint {
    fn printers(&mut self) -> Vec<Printer> {
        system::printers()
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        let file = job_file(job.pdf)?;
        let r = system::print(job, &file);
        let to = job.printer.unwrap_or("the default printer");
        r.map(|note| format!("Sent “{}” to {to}{note}", job.title))
    }

    fn has_setup(&self) -> bool {
        true
    }

    fn setup(&mut self, printer: Option<&str>) -> Result<(), String> {
        system::setup(printer)
    }
}

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
    std::fs::write(&path, pdf).map_err(|e| format!("can't write {}: {e}", path.display()))?;
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

/// `lpstat -e` / `lpstat -a` → the printer names (the first word of each line).
#[cfg(any(test, not(windows)))]
fn lpstat_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = vec![];
    for name in text.lines().filter_map(|l| l.split_whitespace().next()) {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// `lpstat -d` → the default printer ("system default destination: NAME"; none: "no system
/// default destination").
#[cfg(any(test, not(windows)))]
fn lpstat_default(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.contains(':'))?;
    let name = line.rsplit_once(':')?.1.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// PowerShell's `<Default>\t<Name>` lines → the printers.
#[cfg(any(test, windows))]
fn windows_printers(text: &str) -> Vec<Printer> {
    text.lines()
        .filter_map(|l| l.trim_end_matches('\r').split_once('\t'))
        .filter(|(_, name)| !name.trim().is_empty())
        .map(|(default, name)| Printer { name: name.trim().to_string(), default: default.trim().eq_ignore_ascii_case("true") })
        .collect()
}

/// `s` as a PowerShell single-quoted string.
#[cfg(any(test, windows))]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(not(windows))]
mod system {
    use super::*;

    pub fn printers() -> Vec<Printer> {
        // `-e` lists every destination (CUPS 2); older systems answer `-a`.
        let names = output(Command::new("lpstat").arg("-e")).or_else(|_| output(Command::new("lpstat").arg("-a"))).map(|t| lpstat_names(&t));
        let default = output(Command::new("lpstat").arg("-d")).ok().and_then(|t| lpstat_default(&t));
        names.unwrap_or_default().into_iter().map(|name| Printer { default: default.as_ref() == Some(&name), name }).collect()
    }

    pub fn print(job: &PrintJob, file: &std::path::Path) -> Result<String, String> {
        let mut c = Command::new("lp");
        if let Some(p) = job.printer {
            c.args(["-d", p]);
        }
        c.args(["-t", job.title]).arg(file);
        let r = output(&mut c);
        // `lp` has spooled its own copy (or failed).
        let _ = std::fs::remove_file(file);
        r.map(|out| Some(out.trim().to_string()).filter(|o| !o.is_empty()).map(|o| format!(" ({o})")).unwrap_or_default())
    }

    pub fn setup(printer: Option<&str>) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            let _ = printer;
            Command::new("open")
                .arg("x-apple.systempreferences:com.apple.Print-Scan-Settings.extension")
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            if Command::new("system-config-printer").spawn().is_ok() {
                return Ok(());
            }
            // CUPS' own pages.
            let url = printer.map_or_else(|| "http://localhost:631/printers/".to_string(), |p| format!("http://localhost:631/printers/{p}"));
            Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(windows)]
mod system {
    use super::*;

    fn powershell(script: &str) -> Command {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(CREATE_NO_WINDOW);
        c
    }

    pub fn printers() -> Vec<Printer> {
        #[cfg(feature = "windows7")]
        let script = "Get-WmiObject Win32_Printer | ForEach-Object { \"$($_.Default)`t$($_.Name)\" }";
        #[cfg(not(feature = "windows7"))]
        let script = "Get-CimInstance Win32_Printer | ForEach-Object { \"$($_.Default)`t$($_.Name)\" }";
        output(&mut powershell(script)).map(|t| windows_printers(&t)).unwrap_or_default()
    }

    pub fn print(job: &PrintJob, file: &std::path::Path) -> Result<String, String> {
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
        #[cfg(feature = "windows7")]
        let mut command = {
            let mut command = Command::new("control.exe");
            command.arg("printers");
            command
        };
        #[cfg(not(feature = "windows7"))]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg("ms-settings:printers");
            command
        };
        command.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpstat_lists_names_and_the_default() {
        let names = lpstat_names("Office_Laser accepting requests since Mon 05 Oct 2026\nPhoto-Inkjet accepting requests since …\n\n");
        assert_eq!(names, ["Office_Laser", "Photo-Inkjet"]);
        assert_eq!(lpstat_names("Office_Laser\nOffice_Laser\n"), ["Office_Laser"]);
        assert_eq!(lpstat_default("system default destination: Photo-Inkjet\n").as_deref(), Some("Photo-Inkjet"));
        assert_eq!(lpstat_default("no system default destination\n"), None);
    }

    #[test]
    fn powershell_lines_become_printers() {
        let got = windows_printers("False\tMicrosoft Print to PDF\r\nTrue\tOffice Laser\r\n\r\nFalse\t \r\n");
        assert_eq!(got, [Printer { name: "Microsoft Print to PDF".into(), default: false }, Printer { name: "Office Laser".into(), default: true }]);
        assert_eq!(ps_quote("C:\\Temp\\it's.pdf"), "'C:\\Temp\\it''s.pdf'");
    }

    #[test]
    fn job_files_hold_the_pdf() {
        let path = job_file(b"%PDF-1.7 test").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7 test");
        let _ = std::fs::remove_file(path);
    }
}
