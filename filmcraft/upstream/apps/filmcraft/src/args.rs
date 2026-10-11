//! The desktop app's command line (`filmcraft --help`).

/// `filmcraft --help`.
pub const USAGE: &str = "\
Usage: filmcraft [options] [project.fcproj | media files...]

Options:
  --control <port>   start the localhost JSON-lines control server
                     (also: FILMCRAFT_CONTROL_PORT)
  --demo             open the demo project
  --empty            start with an empty project
  --recover          recover the newest unsaved changes without asking
  --no-recover       start without asking about unsaved changes
  --data-dir <dir>   folder for auto-save, crash recovery and logs
                     (also: FILMCRAFT_DATA_DIR)
  --version          print the version and exit
  -h, --help         print this help and exit
";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub enum Cli {
    Run(Launch),
    Help,
    Version,
}

/// Options of a normal start.
#[derive(Debug, PartialEq)]
pub struct Launch {
    pub control_port: Option<u16>,
    /// A project and / or media files to open.
    pub files: Vec<String>,
    pub demo: bool,
    /// `--demo` / `--empty` given: skip Settings ▸ General ▸ At Startup.
    pub startup_flag: bool,
    pub recover: Option<bool>,
    pub data_dir: Option<std::path::PathBuf>,
}

/// Parse the arguments after the program name. `env_control_port` is the value of
/// `FILMCRAFT_CONTROL_PORT`, used when `--control` is not given (an empty value counts as unset).
///
/// An option we don't know, or one without a usable value, is an error: opening a window for a
/// mistyped flag helps nobody, and starting without the control server that was asked for leaves
/// a script waiting for it. That holds for the whole line, so `--version --bogus` is an error
/// too, and for a port from the environment exactly as for one from `--control`.
pub fn parse(env_control_port: Option<String>, args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut l = Launch { control_port: None, files: Vec::new(), demo: true, startup_flag: false, recover: None, data_dir: None };
    let (mut help, mut version) = (false, false);
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => help = true,
            "--version" => version = true,
            "--control" => {
                let port = args.next().ok_or("--control needs a port number")?;
                l.control_port = Some(port.parse().map_err(|_| format!("--control needs a port number, got `{port}`"))?);
            }
            "--demo" => (l.demo, l.startup_flag) = (true, true),
            "--empty" => (l.demo, l.startup_flag) = (false, true),
            "--recover" => l.recover = Some(true),
            "--no-recover" => l.recover = Some(false),
            "--data-dir" => l.data_dir = Some(args.next().map(std::path::PathBuf::from).ok_or("--data-dir needs a folder")?),
            // everything after `--` is a file, whatever it looks like
            "--" => l.files.extend(args.by_ref()),
            // macOS adds a process serial number when it launches an app bundle
            _ if a.starts_with("-psn_") => {}
            _ if a.len() > 1 && a.starts_with('-') => return Err(format!("unknown option `{a}`")),
            _ => l.files.push(a),
        }
    }
    if help {
        return Ok(Cli::Help);
    }
    if version {
        return Ok(Cli::Version);
    }
    if l.control_port.is_none()
        && let Some(port) = env_control_port.as_deref().map(str::trim).filter(|p| !p.is_empty())
    {
        l.control_port = Some(port.parse().map_err(|_| format!("FILMCRAFT_CONTROL_PORT needs a port number, got `{port}`"))?);
    }
    Ok(Cli::Run(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_env(env: Option<&str>, args: &[&str]) -> Result<Cli, String> {
        parse(env.map(str::to_string), args.iter().map(|a| a.to_string()))
    }

    fn parse_args(args: &[&str]) -> Result<Cli, String> {
        parse_env(None, args)
    }

    fn launch(args: &[&str]) -> Launch {
        match parse_args(args) {
            Ok(Cli::Run(l)) => l,
            other => panic!("{args:?} should start the app, got {other:?}"),
        }
    }

    /// `filmcraft --help` used to open the app with "--help" as a media file to import.
    #[test]
    fn help_and_version_do_not_start_the_app() {
        assert_eq!(parse_args(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["-h"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["--empty", "--help"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["--version"]), Ok(Cli::Version));
        assert_eq!(parse_args(&["--version", "--help"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["a.mp4", "--version"]), Ok(Cli::Version));
        for flag in ["--control", "--demo", "--empty", "--recover", "--no-recover", "--data-dir", "--version", "--help"] {
            assert!(USAGE.contains(flag), "usage lists {flag}");
        }
    }

    #[test]
    fn unknown_options_are_errors() {
        assert_eq!(parse_args(&["--frobnicate"]), Err("unknown option `--frobnicate`".to_string()));
        assert!(parse_args(&["a.mp4", "-x"]).is_err());
        assert!(parse_args(&["--Help"]).is_err());
        assert!(parse_args(&["--control"]).is_err(), "missing port");
        assert!(parse_args(&["--control", "abc"]).unwrap_err().contains("abc"));
        assert!(parse_args(&["--control", "70000"]).is_err(), "not a port");
        assert!(parse_args(&["--data-dir"]).is_err(), "missing folder");
        // the whole line is checked, whatever comes first
        assert_eq!(parse_args(&["--version", "--bogus"]), Err("unknown option `--bogus`".to_string()));
        assert_eq!(parse_args(&["--bogus", "--version"]), Err("unknown option `--bogus`".to_string()));
        assert_eq!(parse_args(&["--help", "--bogus"]), Err("unknown option `--bogus`".to_string()));
        assert!(parse_args(&["--help", "--control"]).is_err());
    }

    /// A bad `FILMCRAFT_CONTROL_PORT` used to start the app without the control server, silently.
    #[test]
    fn a_bad_port_in_the_environment_is_an_error_like_a_bad_control_flag() {
        for bad in ["abc", "70000", "-1", "98 76", "9876x", "\u{fffd}"] {
            let e = parse_env(Some(bad), &[]).unwrap_err();
            assert!(e.contains("FILMCRAFT_CONTROL_PORT") && e.contains(bad), "{bad}: {e}");
            assert!(parse_env(Some(bad), &["--empty", "a.mp4"]).is_err(), "{bad}");
            // the flag wins, so the environment's value is not looked at; nor for --help / --version
            assert_eq!(parse_env(Some(bad), &["--control", "9"]).map(|c| matches!(c, Cli::Run(l) if l.control_port == Some(9))), Ok(true), "{bad}");
            assert_eq!(parse_env(Some(bad), &["--help"]), Ok(Cli::Help));
            assert_eq!(parse_env(Some(bad), &["--version"]), Ok(Cli::Version));
        }
        // unset or empty: no control server
        assert_eq!(launch(&[]).control_port, None);
        assert_eq!(parse_env(Some(""), &[]), Ok(Cli::Run(launch(&[]))));
        assert_eq!(parse_env(Some("  "), &[]), Ok(Cli::Run(launch(&[]))));
        // a good one is the default
        assert_eq!(parse_env(Some("1234"), &[]), Ok(Cli::Run(Launch { control_port: Some(1234), ..launch(&[]) })));
        assert_eq!(parse_env(Some("1234"), &["--control", "9"]).map(|c| matches!(c, Cli::Run(l) if l.control_port == Some(9))), Ok(true));
    }

    #[test]
    fn known_options_and_files_parse_as_before() {
        let none = launch(&[]);
        assert_eq!(none, Launch { control_port: None, files: vec![], demo: true, startup_flag: false, recover: None, data_dir: None });
        let l = launch(&["--control", "9876", "--empty", "--no-recover", "--data-dir", "/tmp/fc", "cut.fcproj", "a.mp4"]);
        assert_eq!(l.control_port, Some(9876));
        assert_eq!((l.demo, l.startup_flag, l.recover), (false, true, Some(false)));
        assert_eq!(l.data_dir, Some(std::path::PathBuf::from("/tmp/fc")));
        assert_eq!(l.files, ["cut.fcproj", "a.mp4"]);
        assert_eq!((launch(&["--demo"]).demo, launch(&["--demo"]).startup_flag, launch(&["--recover"]).recover), (true, true, Some(true)));
        // `--` ends the options; a lone `-` and macOS' process serial number are not options
        assert_eq!(launch(&["--", "--odd name.mov", "-h"]).files, ["--odd name.mov", "-h"]);
        assert_eq!(launch(&["-", "-psn_0_12345"]).files, ["-"]);
    }
}
