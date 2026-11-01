use crate::api::{Profile, RepoStats, SocialLink};

pub struct Stats {
    pub repos: Option<RepoStats>,
    pub prs: Option<u64>,
    pub issues: Option<u64>,
    pub socials: Vec<SocialLink>,
}

pub struct Layout {
    pub color: bool,
    pub width: usize,
    pub rows: usize,
}

// User-controlled profile text must never introduce terminal control sequences.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\n' | '\r' | '\t' => Some(' '),
            ch if ch.is_control()
                || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                None
            }
            ch => Some(ch),
        })
        .collect()
}

fn cells(ch: char) -> usize {
    match ch as u32 {
        0x0300..=0x036f
        | 0x1ab0..=0x1aff
        | 0x1dc0..=0x1dff
        | 0x200b..=0x200f
        | 0x20d0..=0x20ff
        | 0xfe00..=0xfe0f
        | 0xfe20..=0xfe2f
        | 0xe0100..=0xe01ef => 0,
        0x1100..=0x115f
        | 0x2329..=0x232a
        | 0x2e80..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe10..=0xfe19
        | 0xfe30..=0xfe6f
        | 0xff01..=0xff60
        | 0xffe0..=0xffe6
        | 0x1f000..=0x1faff
        | 0x20000..=0x3fffd => 2,
        _ => 1,
    }
}

fn clip(text: &str, width: usize) -> String {
    let text = clean(text);
    if text.chars().map(cells).sum::<usize>() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let size = cells(ch);
        if used + size >= width {
            break;
        }
        used += size;
        out.push(ch);
    }
    out.push('…');
    out
}

fn paint(text: &str, rgb: &str, color: bool, bold: bool) -> String {
    if color {
        format!(
            "\x1b[{}38;2;{rgb}m{text}\x1b[0m",
            if bold { "1;" } else { "" }
        )
    } else {
        text.into()
    }
}

fn profile_rows(profile: &Profile, stats: &Stats) -> Vec<(String, String)> {
    let count = |value: Option<u64>| value.map(|v| v.to_string()).unwrap_or_else(|| "—".into());
    let mut rows: Vec<_> = [
        ("ID", profile.id.to_string()),
        ("Stars", count(stats.repos.as_ref().map(|s| s.stars))),
        ("PRs", count(stats.prs)),
        ("Followers", profile.followers.to_string()),
        ("Following", profile.following.to_string()),
        ("Repos", profile.repos.to_string()),
        ("Issues", count(stats.issues)),
        ("Joined", profile.joined.clone()),
        (
            "Language",
            stats
                .repos
                .as_ref()
                .map(|s| s.language.clone())
                .unwrap_or_else(|| "—".into()),
        ),
        ("Location", profile.location.clone()),
    ]
    .into_iter()
    .filter(|(label, _)| !profile.is_org || !matches!(*label, "PRs" | "Issues" | "Following"))
    .map(|(label, value)| (label.to_string(), value))
    .collect();
    if !profile.name.trim().is_empty() && profile.name != profile.login {
        rows.insert(1, ("Name".into(), profile.name.clone()));
    }
    if profile.is_org {
        rows.insert(1, ("Type".into(), "Organization".into()));
    }
    for (label, value) in [("Company", &profile.company), ("Website", &profile.website)] {
        if !value.trim().is_empty() {
            rows.push((label.into(), value.clone()));
        }
    }
    let mut seen = vec![profile.website.trim_end_matches('/').to_string()];
    let has_twitter = stats.socials.iter().any(|link| {
        matches!(link.provider.as_str(), "twitter" | "x")
            || link.url.starts_with("https://twitter.com/")
            || link.url.starts_with("https://x.com/")
    });
    if !profile.twitter.trim().is_empty() && !has_twitter {
        rows.push(("Twitter".into(), format!("@{}", profile.twitter)));
    }
    for link in &stats.socials {
        let url = link.url.trim_end_matches('/');
        if seen.iter().any(|existing| existing == url) {
            continue;
        }
        seen.push(url.to_string());
        let provider = clean(&link.provider);
        let label = match provider.as_str() {
            "twitter" | "x" => "Twitter".into(),
            "youtube" => "YouTube".into(),
            "linkedin" => "LinkedIn".into(),
            "generic" | "" => "Link".into(),
            _ => {
                let mut label = provider;
                if let Some(first) = label.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                label
            }
        };
        rows.push((label, link.url.clone()));
    }
    rows
}

pub fn render(profile: &Profile, stats: &Stats, layout: &Layout, avatar: Option<&[u8]>) -> String {
    let rows = profile_rows(profile, stats);
    let text_height = 2 + rows.len() + if profile.bio.is_empty() { 0 } else { 2 };
    let has_image =
        avatar.is_some() && layout.width >= 68 && layout.rows >= text_height.max(13) + 2;
    let offset = if has_image {
        31
    } else {
        2.min(layout.width.saturating_sub(1))
    };
    let available = layout.width.saturating_sub(offset + 1).max(1);
    let prefix = " ".repeat(offset);
    let mut lines = Vec::new();
    let title = clip(&profile.login, available);
    lines.push(paint(&title, "203;166;247", layout.color, true));
    lines.push(paint(
        &"─".repeat(title.chars().map(cells).sum()),
        "205;214;244",
        layout.color,
        false,
    ));
    const KEY_COLORS: [&str; 7] = [
        "137;220;235",
        "166;227;161",
        "250;179;135",
        "180;190;254",
        "116;199;236",
        "249;226;175",
        "203;166;247",
    ];
    for (index, (label, value)) in rows.into_iter().enumerate() {
        let rgb = KEY_COLORS[index % KEY_COLORS.len()];
        let label = clip(&label, 11);
        let padding = " ".repeat(12 - label.chars().map(cells).sum::<usize>());
        let value = if value.is_empty() { "—" } else { &value };
        if available > 13 {
            lines.push(format!(
                "{}{}",
                paint(&format!("{label}{padding}"), rgb, layout.color, true),
                paint(
                    &clip(value, available - 12),
                    "205;214;244",
                    layout.color,
                    false
                )
            ));
        } else {
            lines.push(paint(
                &clip(&format!("{label}: {value}"), available),
                rgb,
                layout.color,
                true,
            ));
        }
    }
    if !profile.bio.is_empty() {
        lines.push(String::new());
        lines.push(paint(
            &clip(&profile.bio, available),
            "205;214;244",
            layout.color,
            false,
        ));
    }
    let height = lines.len().max(if has_image { 13 } else { 0 });
    let mut output = String::from("\n");
    for line in &lines {
        output.push_str(&format!("{prefix}{line}\n"));
    }
    for _ in lines.len()..height {
        output.push('\n');
    }
    if let Some(bytes) = avatar.filter(|_| has_image) {
        // Reserve rows first, then draw, so scrolling cannot overwrite the image.
        output.push_str(&format!("\x1b[{height}A\r\x1b[2C\x1b7"));
        let encoded = base64(bytes);
        let chunks: Vec<_> = encoded.as_bytes().chunks(4096).collect();
        for (index, chunk) in chunks.iter().enumerate() {
            let more = u8::from(index + 1 < chunks.len());
            if index == 0 {
                output.push_str("\x1b_Ga=T,f=100,t=d,c=26,r=13,C=1,q=2,");
            } else {
                output.push_str("\x1b_Gq=2,");
            }
            output.push_str(&format!(
                "m={more};{}\x1b\\",
                std::str::from_utf8(chunk).unwrap()
            ));
        }
        output.push_str(&format!("\x1b8\r\x1b[{height}B"));
    }
    output.push('\n');
    output
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as usize) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as usize) << 8)
            | chunk.get(2).copied().unwrap_or(0) as usize;
        result.push(TABLE[(n >> 18) & 63] as char);
        result.push(TABLE[(n >> 12) & 63] as char);
        result.push(if chunk.len() > 1 {
            TABLE[(n >> 6) & 63] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            TABLE[n & 63] as char
        } else {
            '='
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_standard_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn clips_unicode_and_removes_terminal_controls() {
        assert_eq!(clean("a\x1b]52;secret\x07\n\u{202e}b"), "a]52;secret b");
        assert_eq!(clip("東京Rust", 5), "東京…");
        assert_eq!(clip("Señor", 4), "Señ…");
        assert_eq!(clip("e\u{301}x", 2), "e\u{301}x");
    }

    fn profile() -> Profile {
        Profile {
            login: "octocat".into(),
            name: String::new(),
            company: String::new(),
            website: String::new(),
            is_org: false,
            id: 1,
            followers: 2,
            following: 3,
            repos: 4,
            joined: "2010-01-04".into(),
            location: "東京".into(),
            twitter: "octocat".into(),
            bio: "Señor 🥷".into(),
        }
    }

    #[test]
    fn kitty_chunks_reconstruct_image_and_leave_cursor_below_profile() {
        let avatar = vec![42; 7000];
        let text = render(
            &profile(),
            &Stats {
                repos: None,
                prs: None,
                issues: None,
                socials: Vec::new(),
            },
            &Layout {
                color: false,
                width: 80,
                rows: 24,
            },
            Some(&avatar),
        );
        let mut payload = String::new();
        let mut parts = text.split("\x1b_G").skip(1).peekable();
        while let Some(part) = parts.next() {
            let (command, _) = part.split_once("\x1b\\").unwrap();
            let (control, data) = command.split_once(';').unwrap();
            assert!(data.len() <= 4096);
            assert_eq!(data.len() % 4, 0);
            assert!(control.contains(if parts.peek().is_some() { "m=1" } else { "m=0" }));
            payload.push_str(data);
        }
        assert_eq!(payload, base64(&avatar));
        assert!(text.contains("a=T,f=100,t=d,c=26,r=13,C=1,q=2,"));
        assert!(text.contains("\x1b[15A\r\x1b[2C\x1b7"));
        assert!(text.ends_with("\x1b8\r\x1b[15B\n"));
    }

    #[test]
    fn narrow_layout_skips_image() {
        let stats = Stats {
            repos: None,
            prs: None,
            issues: None,
            socials: Vec::new(),
        };
        let avatar = b"fixture";
        let layout = Layout {
            color: false,
            width: 40,
            rows: 24,
        };
        let text = render(&profile(), &stats, &layout, Some(avatar));
        assert!(!text.contains('\x1b'));
        assert!(text.starts_with("\n  octocat\n"));
    }

    #[test]
    fn social_labels_are_safe_deduplicated_and_accept_unknown_providers() {
        let mut profile = profile();
        profile.website = "https://example.com".into();
        let link = |provider: &str, url: &str| SocialLink {
            provider: provider.into(),
            url: url.into(),
        };
        let stats = Stats {
            repos: None,
            prs: None,
            issues: None,
            socials: vec![
                link("generic", "https://example.com/"),
                link("twitter", "https://x.com/octocat"),
                link("twitter", "https://x.com/octocat/"),
                link("future\x1b\x07provider", "https://example.net/new"),
            ],
        };
        let text = render(
            &profile,
            &stats,
            &Layout {
                color: false,
                width: 80,
                rows: 24,
            },
            None,
        );
        assert_eq!(text.matches("Twitter").count(), 1);
        assert_eq!(text.matches("https://example.com").count(), 1);
        assert!(text.contains("https://example.net/new"));
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
        assert!(!text.contains("Company"));
        assert!(!text.contains("Email"));
    }

    #[test]
    fn tall_profiles_use_text_layout_instead_of_misplacing_images() {
        let stats = Stats {
            repos: None,
            prs: None,
            issues: None,
            socials: Vec::new(),
        };
        let text = render(
            &profile(),
            &stats,
            &Layout {
                color: false,
                width: 80,
                rows: 16,
            },
            Some(b"fixture"),
        );
        assert!(!text.contains('\x1b'));
        assert!(text.starts_with("\n  octocat\n"));
    }
}
