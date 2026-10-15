//! Desktop printer transport. VectorCraft composes every page and copy before submission;
//! craft-print owns discovery and spooling. Windows 7 keeps its viewer-based transport because
//! Windows.Data.Pdf is unavailable there.
#[cfg(not(all(windows, any(feature = "windows7", target_vendor = "win7"))))]
use std::process::Command;
use vectorcraft_ui_egui::print::{PrintJob, PrintService, Printer};

#[cfg(all(windows, any(feature = "windows7", target_vendor = "win7")))]
mod windows7;
#[cfg(all(windows, any(feature = "windows7", target_vendor = "win7")))]
use windows7 as system;

pub struct SystemPrint;
impl PrintService for SystemPrint {
    fn printers(&mut self) -> Vec<Printer> {
        system::printers()
    }
    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        let result = system::print(job);
        let to = job.printer.unwrap_or("the default printer");
        result.map(|note| format!("Sent “{}” to {to}{note}", job.title))
    }
    fn shared_print(&self) -> Option<vectorcraft_ui_egui::print::SharedPrintFn> {
        Some(std::sync::Arc::new(|job| SystemPrint.print(job)))
    }
    fn has_setup(&self) -> bool {
        true
    }
    fn setup(&mut self, printer: Option<&str>) -> Result<(), String> {
        system::setup(printer)
    }
}

#[cfg(not(all(windows, any(feature = "windows7", target_vendor = "win7"))))]
mod system {
    use super::*;
    pub fn printers() -> Vec<Printer> {
        craft_print::printers().into_iter().map(|p| Printer { name: p.name, default: p.default }).collect()
    }
    pub fn print(job: &PrintJob) -> Result<String, String> {
        // Copies, layout, marks and colour management are already in the PDF. Preserve its
        // physical sheet size and the queue's duplex default without printing copies twice.
        let settings = craft_print::Job { printer: job.printer.map(str::to_string), title: job.title.into(), ..Default::default() };
        craft_print::submit(job.pdf, &settings).map(|note| if note.is_empty() { note } else { format!(" ({note})") }).map_err(|e| e.to_string())
    }
    pub fn setup(printer: Option<&str>) -> Result<(), String> {
        #[cfg(windows)]
        {
            let _ = printer;
            Command::new("explorer").arg("ms-settings:printers").spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        #[cfg(target_os = "macos")]
        {
            let _ = printer;
            Command::new("open")
                .arg("x-apple.systempreferences:com.apple.Print-Scan-Settings.extension")
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            if Command::new("system-config-printer").spawn().is_ok() {
                return Ok(());
            }
            let url = printer.map_or_else(|| "http://localhost:631/printers/".to_string(), |p| format!("http://localhost:631/printers/{p}"));
            Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
    }
}
