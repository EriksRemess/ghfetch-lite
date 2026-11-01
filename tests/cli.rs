#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ghfetch-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let curl = dir.join("curl");
        fs::write(&curl, r##"#!/bin/sh
config=$(cat)
case "$config" in
  *'Authorization: Bearer fixture_token'*) ;;
  *) printf 'missing authentication config' >&2; exit 1 ;;
esac
for argument do
  case "$argument" in *fixture_token*) printf 'token leaked in arguments' >&2; exit 1 ;; esac
  url="$argument"
done
case "$url" in
  'https://api.github.com/users/missing')
    printf '%s\n404' '{"message":"Not Found"}' ;;
  'https://api.github.com/users/octocat')
    printf '%s\n200' '{"login":"octocat","id":1,"followers":99,"following":157,"public_repos":2,"created_at":"2010-01-04T00:00:00Z","location":"Saldus, Latvia","twitter_username":null,"bio":"Señor Developer \u001b]52;bad\u0007"}' ;;
  'https://api.github.com/users/rich')
    printf '%s\n200' '{"login":"rich","type":"User","id":2,"name":"Rich Profile","company":"@vercel | @nuxt","blog":"https://example.com","email":"hello@example.com","twitter_username":"rich","followers":1,"following":2,"public_repos":3,"created_at":"2010-01-04","bio":"Hello"}' ;;
  'https://api.github.com/users/ghostty-org')
    printf '%s\n200' '{"login":"ghostty-org","type":"Organization","id":3,"followers":100,"following":0,"public_repos":6,"created_at":"2022-01-01","bio":null}' ;;
  'https://api.github.com/orgs/ghostty-org')
    printf '%s\n200' '{"name":"Ghostty","description":"A new terminal","blog":"https://ghostty.org","email":"contact@example.com","location":"United States"}' ;;
  'https://api.github.com/users/octocat/social_accounts?per_page=100&page=1')
    printf '%s\n200' '[]' ;;
  'https://api.github.com/users/rich/social_accounts?per_page=100&page=1')
    if [ "$FIXTURE_PARTIAL" = 1 ]; then
      printf '%s\n403' '{"message":"rate limited"}'
    else
      printf '%s\n200' '[{"provider":"twitter","url":"https://x.com/rich"},{"provider":"bluesky","url":"https://bsky.app/profile/rich.example"},{"provider":"youtube","url":"https://youtube.com/@rich"},{"provider":"customservice","url":"https://custom.example/rich"}]'
    fi ;;
  'https://api.github.com/users/'*'/repos?type=owner&per_page=100&page=1')
    printf '%s\n200' '[{"fork":false,"stargazers_count":23,"language":"Rust"},{"fork":true,"stargazers_count":100,"language":"C"}]' ;;
  *'type%3Apr'*)
    if [ "$FIXTURE_PARTIAL" = 1 ]; then
      printf '%s\n200' '{"total_count":61,"incomplete_results":true}'
    else
      printf '%s\n200' '{"total_count":61,"incomplete_results":false}'
    fi ;;
  *'type%3Aissue'*)
    printf '%s\n200' '{"total_count":11,"incomplete_results":false}' ;;
  *) printf 'unexpected request: %s' "$url" >&2; exit 1 ;;
esac
"##).unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
        Self { dir }
    }

    fn run(&self, args: &[&str], partial: bool) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_ghfetch-lite"))
            .args(args)
            .env("PATH", path)
            .env("GH_TOKEN", "fixture_token")
            .env("COLUMNS", "80")
            .env("TERM", "xterm-kitty")
            .env("TERM_PROGRAM", "ghostty")
            .env("FIXTURE_PARTIAL", if partial { "1" } else { "0" })
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn full_cli_fetches_and_renders_without_escapes_when_piped() {
    let fixture = Fixture::new();
    let output = fixture.run(&["octocat", "--image", "kitty"], false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text,
        "\n  octocat\n  ───────\n  ID          1\n  Stars       23\n  PRs         61\n  Followers   99\n  Following   157\n  Repos       2\n  Issues      11\n  Joined      2010-01-04\n  Language    Rust\n  Location    Saldus, Latvia\n  \n  Señor Developer ]52;bad\n\n"
    );
    assert!(!text.contains('\x1b'));
}

#[test]
fn incomplete_search_is_unavailable_and_profile_survives() {
    let output = Fixture::new().run(&["octocat"], true);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("PRs         —"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("incomplete results"));
}

#[test]
fn missing_user_returns_failure_without_profile() {
    let output = Fixture::new().run(&["missing"], false);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 404: Not Found"));
}

#[test]
fn narrow_output_stays_inside_requested_width() {
    let output = Fixture::new().run(&["octocat", "--width", "20", "--no-color"], false);
    assert!(output.status.success());
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        assert!(line.chars().count() <= 20, "line exceeds width: {line}");
    }
}

#[test]
fn populated_profile_renders_contacts_and_social_providers() {
    let output = Fixture::new().run(&["rich", "--width", "100"], false);
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Name        Rich Profile",
        "Company     @vercel | @nuxt",
        "Website     https://example.com",
        "Twitter     https://x.com/rich",
        "Bluesky     https://bsky.app/profile/rich.example",
        "YouTube     https://youtube.com/@rich",
        "Customserv… https://custom.example/rich",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert_eq!(text.matches("Twitter").count(), 1);
    assert!(!text.contains("Email"));
    assert!(!text.contains("hello@example.com"));
}

#[test]
fn social_failure_preserves_profile_contacts_and_legacy_twitter() {
    let output = Fixture::new().run(&["rich"], true);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Website     https://example.com"));
    assert!(text.contains("Twitter     @rich"));
    assert!(!text.contains("Bluesky"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("social accounts: HTTP 403"));
}

#[test]
fn organization_uses_description_and_omits_individual_user_fields() {
    let output = Fixture::new().run(&["ghostty-org"], false);
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Type        Organization",
        "Name        Ghostty",
        "Website     https://ghostty.org",
        "A new terminal",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    for absent in [
        "PRs",
        "Issues",
        "Following",
        "Twitter",
        "Email",
        "contact@example.com",
    ] {
        assert!(!text.contains(absent));
    }
}
