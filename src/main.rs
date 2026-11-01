mod api;
mod json;
mod render;

use api::{Client, Profile};
use render::{Layout, Stats};
use std::{
    env,
    io::{self, IsTerminal, Write},
    process::ExitCode,
    thread,
};

const HELP: &str = "ghfetch-lite — GitHub profiles in your terminal

Usage: ghfetch-lite [OPTIONS] USERNAME

Options:
  --no-image      Disable the Kitty avatar
  --color WHEN    auto, always, or never (default: auto)
  --no-color      Disable colors (also respects NO_COLOR)
  --width COLS    Override terminal width (default: detected, then COLUMNS, then 80)
  -h, --help     Show this help
  -V, --version  Show version

Requires curl for HTTPS; there are no Cargo dependencies.
Set GH_TOKEN or GITHUB_TOKEN to increase GitHub's API rate limit.
Images require a supported terminal; redirected output is always text-only.
";

struct Options {
    username: String,
    image: bool,
    color: String,
    width: Option<usize>,
}

enum Action {
    Run(Options),
    Help,
    Version,
}

fn options(args: impl IntoIterator<Item = String>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut username = None;
    let mut image = true;
    let mut color = "auto".to_string();
    let mut width = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            "--no-image" => image = false,
            "--no-color" => color = "never".into(),
            "--color" => {
                color = args
                    .next()
                    .ok_or("--color expects auto, always, or never")?;
                if !matches!(color.as_str(), "auto" | "always" | "never") {
                    return Err("--color expects auto, always, or never".into());
                }
            }
            "--width" => {
                let value = args
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|v| (1..=1000).contains(v))
                    .ok_or("--width expects a number from 1 to 1000")?;
                width = Some(value);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ if username.is_some() => return Err("expected just one GitHub username".into()),
            _ => username = Some(arg),
        }
    }
    let username = username.ok_or("provide a GitHub username; try --help")?;
    if !api::valid_username(&username) {
        return Err("invalid GitHub username".into());
    }
    Ok(Action::Run(Options {
        username,
        image,
        color,
        width,
    }))
}

fn terminal_size() -> Option<(usize, usize)> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        #[repr(C)]
        struct WindowSize {
            rows: u16,
            cols: u16,
            x: u16,
            y: u16,
        }
        unsafe extern "C" {
            fn ioctl(fd: std::ffi::c_int, request: std::ffi::c_ulong, ...) -> std::ffi::c_int;
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        const GET_SIZE: std::ffi::c_ulong = 0x5413;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        const GET_SIZE: std::ffi::c_ulong = 0x40087468;
        let mut size = WindowSize {
            rows: 0,
            cols: 0,
            x: 0,
            y: 0,
        };
        // SAFETY: TIOCGWINSZ writes exactly this four-u16 struct; fd 1 is stdout.
        if unsafe { ioctl(1, GET_SIZE, &mut size) } == 0 && size.cols > 0 && size.rows > 0 {
            return Some((size.cols as usize, size.rows as usize));
        }
    }
    None
}

fn optional<T>(label: &str, result: Result<T, String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("ghfetch-lite: {label}: {}", render::clean(&error));
            None
        }
    }
}

fn output_width(
    requested: Option<usize>,
    detected: Option<usize>,
    columns: Option<usize>,
) -> usize {
    let width = requested
        .or(detected)
        .or(columns)
        .unwrap_or(80)
        .clamp(1, 1000);
    width.min(detected.unwrap_or(width).max(1))
}

fn run(options: Options) -> Result<(), String> {
    let client = Client::new()?;
    let mut profile = Profile::from_json(&client.get(&format!("users/{}", options.username))?)?;
    if profile.is_org {
        if let Some(details) = optional(
            "organization details",
            client.get(&format!("orgs/{}", profile.login)),
        ) {
            profile.update_organization(&details);
        }
    }
    let terminal = io::stdout().is_terminal();
    let size = terminal_size();
    let width = output_width(
        options.width,
        size.map(|s| s.0),
        env::var("COLUMNS").ok().and_then(|s| s.parse().ok()),
    );
    let show_image =
        options.image && terminal && width >= 68 && size.map(|s| s.1).unwrap_or(24) >= 18;
    let color = match options.color.as_str() {
        "always" => true,
        "never" => false,
        _ => {
            terminal
                && env::var_os("NO_COLOR").is_none()
                && env::var("TERM").is_ok_and(|t| t != "dumb")
        }
    };
    let (repos, prs, issues, avatar, socials) = thread::scope(|scope| {
        let repos = scope.spawn(|| client.repositories(&profile.login));
        let prs = scope.spawn(|| {
            if profile.is_org {
                Ok(None)
            } else {
                client.authored(&profile.login, "pr").map(Some)
            }
        });
        let issues = scope.spawn(|| {
            if profile.is_org {
                Ok(None)
            } else {
                client.authored(&profile.login, "issue").map(Some)
            }
        });
        let socials = scope.spawn(|| {
            if profile.is_org {
                Ok(Vec::new())
            } else {
                client.social_accounts(&profile.login)
            }
        });
        let avatar = scope.spawn(|| {
            if show_image {
                client.avatar(profile.id).map(Some)
            } else {
                Ok(None)
            }
        });
        (
            repos.join(),
            prs.join(),
            issues.join(),
            avatar.join(),
            socials.join(),
        )
    });
    let stats = Stats {
        repos: optional("repository statistics", repos.map_err(|_| "worker failed")?),
        prs: optional("pull requests", prs.map_err(|_| "worker failed")?).flatten(),
        issues: optional("issues", issues.map_err(|_| "worker failed")?).flatten(),
        socials: optional("social accounts", socials.map_err(|_| "worker failed")?)
            .unwrap_or_default(),
    };
    let avatar = optional("avatar", avatar.map_err(|_| "worker failed")?).flatten();
    let avatar = avatar.filter(|bytes| {
        let supported = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
        if !supported {
            eprintln!("ghfetch-lite: avatar is not PNG; using text layout");
        }
        supported
    });
    let output = render::render(
        &profile,
        &stats,
        &Layout {
            color,
            width,
            rows: size.map(|s| s.1).unwrap_or(24),
        },
        avatar.as_deref(),
    );
    match io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("could not write output: {error}")),
    }
}

fn main() -> ExitCode {
    let result = match options(env::args().skip(1)) {
        Ok(Action::Help) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Action::Version) => {
            println!("ghfetch-lite {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Action::Run(options)) => run(options),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ghfetch-lite: {}", render::clean(&error));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_overrides_cannot_exceed_the_physical_viewport() {
        assert_eq!(output_width(Some(100), Some(80), Some(120)), 80);
        assert_eq!(output_width(Some(80), Some(60), None), 60);
        assert_eq!(output_width(Some(40), Some(80), None), 40);
        assert_eq!(output_width(None, Some(80), Some(120)), 80);
        // Without a detected viewport, preserve overrides and the existing fallback order.
        assert_eq!(output_width(Some(100), None, Some(80)), 100);
        assert_eq!(output_width(None, None, Some(100)), 100);
        assert_eq!(output_width(None, None, None), 80);
    }

    #[test]
    fn cli_rejects_bad_arguments_without_network() {
        for args in [
            vec![],
            vec!["--width", "0", "octocat"],
            vec!["--color", "bad", "octocat"],
            vec!["--image"],
            vec!["octocat", "--image", "kitty"],
            vec!["a", "b"],
            vec!["../user"],
        ] {
            assert!(options(args.into_iter().map(String::from)).is_err());
        }
        assert!(matches!(options(["--help".into()]), Ok(Action::Help)));
        assert!(matches!(
            options(["--no-image".into(), "octocat".into()]),
            Ok(Action::Run(_))
        ));
    }

    #[test]
    fn images_are_enabled_by_default_and_can_be_disabled() {
        for (args, expected) in [
            (vec!["octocat"], true),
            (vec!["octocat", "--no-image"], false),
        ] {
            let Action::Run(options) = options(args.into_iter().map(String::from)).unwrap() else {
                panic!("expected run")
            };
            assert_eq!(options.image, expected);
        }
    }
}
