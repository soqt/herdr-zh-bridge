// No console window for the release app on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::{ExitCode, Stdio};

use herdr_zh_bridge::{Options, ui};

const USAGE: &str = "用法: herdr-zh-bridge [--target <终端地址>] [--detach] [--preview] [--smoke-test] [--version]";

fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target" => options.target = args.next().ok_or("--target 需要终端地址")?,
            "--detach" => options.detach = true,
            "--preview" => options.preview = true,
            "--smoke-test" => options.smoke_test = true,
            "--version" | "-V" => return Ok(None),
            "--help" | "-h" => return Err(String::new()),
            // Finder may pass a process serial number to older bundles.
            other if other.starts_with("-psn_") => {}
            other => return Err(format!("未知参数：{other}")),
        }
    }
    Ok(Some(options))
}

fn detach(options: &Options) -> std::io::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if !options.target.is_empty() {
        command.args(["--target", &options.target]);
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map(|_| ())
}

fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("herdr-zh-bridge {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if options.detach {
        return match detach(&options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("无法启动中文桥：{error}");
                ExitCode::FAILURE
            }
        };
    }
    ui::run(options);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Options>, String> {
        parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        let options = args(&["--target", "t3", "--detach"]).unwrap().unwrap();
        assert_eq!((options.target.as_str(), options.detach), ("t3", true));
        assert!(args(&["--version"]).unwrap().is_none());
        assert!(args(&["--target"]).is_err());
        assert!(args(&["--bogus"]).is_err());
        assert!(args(&["-psn_0_1234"]).unwrap().is_some());
    }
}
