use crate::json::{self, Value};
use std::{
    env,
    io::Write,
    process::{Command, Stdio},
};

pub struct Client {
    token: Option<String>,
}

impl Client {
    pub fn new() -> Result<Self, String> {
        let token = env::var("GH_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()));
        if token.as_ref().is_some_and(|s| {
            !s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        }) {
            return Err("GH_TOKEN/GITHUB_TOKEN contains invalid characters".into());
        }
        Ok(Self { token })
    }

    fn download(&self, url: &str, api: bool) -> Result<Vec<u8>, String> {
        let mut command = Command::new("curl");
        command.args([
            "-q",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--max-filesize",
            "8388608",
            "--user-agent",
            "ghfetch-lite/0.1.0",
            "--write-out",
            "\n%{http_code}",
        ]);
        if api {
            command.args([
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                "X-GitHub-Api-Version: 2022-11-28",
            ]);
        }
        // Send credentials through stdin, never the process argument list.
        let config = if api {
            self.token
                .as_ref()
                .map(|token| format!("header = \"Authorization: Bearer {token}\"\n"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut child = command
            .args(["--config", "-", "--url", url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start curl (install curl and put it on PATH): {e}"))?;
        let input_result = child.stdin.take().unwrap().write_all(config.as_bytes());
        let output = child
            .wait_with_output()
            .map_err(|e| format!("could not read curl output: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "HTTPS request failed: {}",
                crate::render::clean(&String::from_utf8_lossy(&output.stderr))
            ));
        }
        input_result.map_err(|e| format!("could not configure curl: {e}"))?;
        let split = output
            .stdout
            .iter()
            .rposition(|&b| b == b'\n')
            .ok_or("missing HTTP status from curl")?;
        let status = std::str::from_utf8(&output.stdout[split + 1..]).unwrap_or("unknown");
        let body = &output.stdout[..split];
        if status != "200" {
            let message = std::str::from_utf8(body)
                .ok()
                .and_then(|s| json::parse(s).ok())
                .and_then(|v| {
                    v.get("message")
                        .and_then(Value::text)
                        .map(crate::render::clean)
                })
                .unwrap_or_else(|| "request rejected".into());
            return Err(format!(
                "HTTP {status}: {message}{}",
                if matches!(status, "403" | "429") {
                    "; GitHub may be rate limiting requests; set GH_TOKEN or try again later"
                } else {
                    ""
                }
            ));
        }
        Ok(body.to_vec())
    }

    pub fn get(&self, path: &str) -> Result<Value, String> {
        let bytes = self.download(&format!("https://api.github.com/{path}"), true)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "GitHub returned invalid UTF-8")?;
        json::parse(text)
    }

    pub fn avatar(&self, id: u64) -> Result<Vec<u8>, String> {
        self.download(
            &format!("https://avatars.githubusercontent.com/u/{id}?s=512&v=4"),
            false,
        )
    }

    pub fn repositories(&self, login: &str) -> Result<RepoStats, String> {
        repository_stats(|page| {
            self.get(&format!(
                "users/{login}/repos?type=owner&per_page=100&page={page}"
            ))
        })
    }

    pub fn authored(&self, login: &str, kind: &str) -> Result<u64, String> {
        let data = self.get(&format!(
            "search/issues?q=author%3A{login}+type%3A{kind}+is%3Apublic&per_page=1"
        ))?;
        if data.get("incomplete_results") != Some(&Value::Bool(false)) {
            return Err("GitHub search returned incomplete results".into());
        }
        required_count(&data, "total_count")
    }

    pub fn social_accounts(&self, login: &str) -> Result<Vec<SocialLink>, String> {
        let mut links = Vec::new();
        for page in 1..=100 {
            let data = self.get(&format!(
                "users/{login}/social_accounts?per_page=100&page={page}"
            ))?;
            let items = data
                .array()
                .ok_or("GitHub returned an invalid social account list")?;
            for item in items {
                let provider = item.get("provider").and_then(Value::text).unwrap_or("");
                let url = item
                    .get("url")
                    .and_then(Value::text)
                    .ok_or("social account is missing its URL")?;
                if !url.starts_with("https://") && !url.starts_with("http://") {
                    return Err("social account URL must use HTTP or HTTPS".into());
                }
                links.push(SocialLink {
                    provider: provider.into(),
                    url: url.into(),
                });
            }
            if items.len() < 100 {
                return Ok(links);
            }
        }
        Err("social account pagination exceeded 100 pages".into())
    }
}

#[derive(Debug)]
pub struct SocialLink {
    pub provider: String,
    pub url: String,
}

pub struct Profile {
    pub login: String,
    pub name: String,
    pub company: String,
    pub website: String,
    pub is_org: bool,
    pub id: u64,
    pub followers: u64,
    pub following: u64,
    pub repos: u64,
    pub joined: String,
    pub location: String,
    pub twitter: String,
    pub bio: String,
}

impl Profile {
    pub fn from_json(data: &Value) -> Result<Self, String> {
        let text = |key| {
            data.get(key)
                .and_then(Value::text)
                .unwrap_or("")
                .to_string()
        };
        let login = text("login");
        if !valid_username(&login) {
            return Err("GitHub returned an invalid login".into());
        }
        Ok(Self {
            login,
            name: text("name"),
            company: text("company"),
            website: text("blog"),
            is_org: text("type") == "Organization",
            id: required_count(data, "id")?,
            followers: required_count(data, "followers")?,
            following: required_count(data, "following")?,
            repos: required_count(data, "public_repos")?,
            joined: text("created_at").chars().take(10).collect(),
            location: text("location"),
            twitter: text("twitter_username"),
            bio: if text("bio").is_empty() {
                text("description")
            } else {
                text("bio")
            },
        })
    }

    pub fn update_organization(&mut self, details: &Value) {
        for (key, field) in [
            ("name", &mut self.name),
            ("company", &mut self.company),
            ("blog", &mut self.website),
            ("location", &mut self.location),
            ("twitter_username", &mut self.twitter),
            ("description", &mut self.bio),
        ] {
            if let Some(value) = details
                .get(key)
                .and_then(Value::text)
                .filter(|s| !s.trim().is_empty())
            {
                *field = value.into();
            }
        }
    }
}

pub fn valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 39
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn required_count(data: &Value, key: &str) -> Result<u64, String> {
    data.get(key)
        .and_then(Value::count)
        .ok_or_else(|| format!("GitHub response is missing numeric {key}"))
}

#[derive(Debug, PartialEq)]
pub struct RepoStats {
    pub stars: u64,
    pub language: String,
}

fn repository_stats(
    mut get: impl FnMut(usize) -> Result<Value, String>,
) -> Result<RepoStats, String> {
    let mut stars = 0u64;
    let mut languages = Vec::<(String, u64)>::new();
    for page in 1..=1000 {
        let response = get(page)?;
        let repos = response
            .array()
            .ok_or("GitHub returned an invalid repository list")?;
        for repo in repos {
            match repo.get("fork") {
                Some(Value::Bool(true)) => continue,
                Some(Value::Bool(false)) => {}
                _ => return Err("GitHub response is missing repository fork status".into()),
            }
            stars = stars
                .checked_add(required_count(repo, "stargazers_count")?)
                .ok_or("star count overflow")?;
            if let Some(language) = repo.get("language").and_then(Value::text) {
                if let Some((_, count)) = languages.iter_mut().find(|(name, _)| name == language) {
                    *count += 1;
                } else {
                    languages.push((language.to_string(), 1));
                }
            }
        }
        if repos.len() < 100 {
            // Alphabetical tie break, independent of API ordering.
            let language = languages
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(name, _)| name)
                .unwrap_or_else(|| "—".into());
            return Ok(RepoStats { stars, language });
        }
    }
    Err("repository pagination exceeded 1000 pages".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_details_fill_contacts_without_erasing_base_fields() {
        let data = json::parse(r#"{"login":"ghostty-org","type":"Organization","id":1,"followers":2,"following":0,"public_repos":3,"name":"Ghostty","blog":"https://ghostty.org","bio":null,"description":"Base description"}"#).unwrap();
        let mut profile = Profile::from_json(&data).unwrap();
        assert!(profile.is_org);
        assert_eq!(profile.bio, "Base description");
        profile.update_organization(&json::parse(r#"{"description":"A new terminal","email":"contact@example.com","blog":null,"name":"   "}"#).unwrap());
        assert_eq!(profile.bio, "A new terminal");
        assert_eq!(profile.website, "https://ghostty.org");
        assert_eq!(profile.name, "Ghostty");
    }

    #[test]
    fn validates_usernames_before_constructing_urls() {
        for name in ["EriksRemess", "a", "a-b", "123"] {
            assert!(valid_username(name));
        }
        for name in ["", "-a", "a-", "a--b", "a/b", "a?b", "a\n", "é"] {
            assert!(!valid_username(name));
        }
        assert!(!valid_username(&"a".repeat(40)));
    }

    #[test]
    fn paginates_and_excludes_forks_with_stable_language_ties() {
        let repo = r#"{"fork":false,"stargazers_count":2,"language":"Rust"}"#;
        let first = format!("[{}]", vec![repo; 100].join(","));
        let stats = repository_stats(|page| json::parse(if page == 1 { &first } else {
            r#"[{"fork":true,"stargazers_count":999,"language":"C"},{"fork":false,"stargazers_count":3,"language":null}]"#
        })).unwrap();
        assert_eq!(
            stats,
            RepoStats {
                stars: 203,
                language: "Rust".into()
            }
        );
        let stats = repository_stats(|_| json::parse(r#"[{"fork":false,"stargazers_count":0,"language":"Rust"},{"fork":false,"stargazers_count":0,"language":"C"}]"#)).unwrap();
        assert_eq!(stats.language, "C");
    }

    #[test]
    fn never_reports_partial_totals_as_complete() {
        let first = format!(
            "[{}]",
            vec![r#"{"fork":false,"stargazers_count":1}"#; 100].join(",")
        );
        assert!(
            repository_stats(|page| if page == 1 {
                json::parse(&first)
            } else {
                Err("rate limited".into())
            })
            .is_err()
        );
        assert!(repository_stats(|_| json::parse(r#"[{"fork":false}]"#)).is_err());
        assert_eq!(
            repository_stats(|_| json::parse("[]")).unwrap(),
            RepoStats {
                stars: 0,
                language: "—".into()
            }
        );
    }
}
