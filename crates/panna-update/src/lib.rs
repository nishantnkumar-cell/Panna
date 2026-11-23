//! The one place Panna talks to the network: a single HTTPS request to GitHub's public
//! Releases API, made only when the user clicks **Check for updates** in the About box.
//!
//! Nothing runs in the background or at start-up, no files or document details are sent
//! (only the usual request headers and `Panna/<version>` as the user agent), and the reply
//! is size-limited and parsed as plain JSON. Every failure becomes an [`Error`] the About box
//! shows in words; offline is an ordinary outcome, not a crash.

use std::time::Duration;

use serde::Deserialize;

/// Where releases are published.
pub const RELEASES_PAGE: &str = "https://github.com/nishantnkumar-cell/Panna/releases";
/// GitHub's public API listing of the same releases (drafts are never listed).
const RELEASES_API: &str = "https://api.github.com/repos/nishantnkumar-cell/Panna/releases?per_page=30";
/// A releases listing is a few kilobytes per release; anything bigger is refused.
const MAX_REPLY: u64 = 2 * 1024 * 1024;
/// Gives up rather than leaving the About box waiting.
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// This is the newest published version (or newer, for a development build).
    UpToDate,
    /// A newer version is published.
    Newer(Release),
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The version, without the tag's leading `v`.
    pub version: String,
    /// The release's page on GitHub, where its downloads are.
    pub page: String,
    /// Marked as a pre-release on GitHub.
    pub prerelease: bool,
}

/// Why the check couldn't finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No connection: offline, blocked by a firewall or proxy, or timed out.
    Offline(String),
    /// GitHub answered with something other than a releases listing.
    BadReply(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Offline(why) => write!(
                f,
                "Couldn't reach GitHub ({why}). Check the connection and try again."
            ),
            Error::BadReply(why) => write!(f, "GitHub's answer couldn't be read ({why})."),
        }
    }
}

/// Asks GitHub whether a version newer than `current` is published. Blocks for up to
/// 15 seconds, so call it off the UI thread.
pub fn check(current: &str) -> Result<Outcome, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .https_only(true)
        .max_redirects(3)
        .user_agent(format!("Panna/{current}"))
        .build()
        .into();
    let mut reply = agent
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => Error::BadReply(format!("status {code}")),
            other => Error::Offline(other.to_string()),
        })?;
    let body = reply
        .body_mut()
        .with_config()
        .limit(MAX_REPLY)
        .read_to_string()
        .map_err(|e| Error::BadReply(e.to_string()))?;
    newest(&body, current)
}

#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Picks the newest release in a GitHub releases listing and compares it with `current`.
/// Tags that aren't versions (`v1.2.3` or `1.2.3`) are skipped, as are drafts.
pub fn newest(listing: &str, current: &str) -> Result<Outcome, Error> {
    let current = parse(current).ok_or_else(|| Error::BadReply(format!("this build's version {current}")))?;
    let listed: Vec<Listed> = serde_json::from_str(listing).map_err(|e| Error::BadReply(e.to_string()))?;
    let best = listed
        .into_iter()
        .filter(|r| !r.draft && r.html_url.starts_with("https://github.com/"))
        .filter_map(|r| parse(&r.tag_name).map(|v| (v, r)))
        .max_by(|(a, _), (b, _)| a.cmp(b));
    Ok(match best {
        Some((version, r)) if version > current => Outcome::Newer(Release {
            version: version.to_string(),
            page: r.html_url,
            prerelease: r.prerelease,
        }),
        _ => Outcome::UpToDate,
    })
}

fn parse(tag: &str) -> Option<semver::Version> {
    let tag = tag.trim();
    semver::Version::parse(tag.strip_prefix(['v', 'V']).unwrap_or(tag)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(tags: &[(&str, bool, bool)]) -> String {
        let items: Vec<String> = tags
            .iter()
            .map(|(tag, draft, pre)| {
                format!(
                    r#"{{"tag_name":"{tag}","html_url":"https://github.com/nishantnkumar-cell/Panna/releases/tag/{tag}","draft":{draft},"prerelease":{pre},"assets":[]}}"#
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    #[test]
    fn finds_a_newer_release() {
        let body = listing(&[
            ("v0.1.0", false, true),
            ("v0.3.0", false, true),
            ("v0.2.0", false, false),
        ]);
        let Ok(Outcome::Newer(r)) = newest(&body, "0.2.0") else {
            panic!("expected a newer release");
        };
        assert_eq!(r.version, "0.3.0");
        assert!(r.prerelease);
        assert!(r.page.ends_with("/v0.3.0"));
    }

    #[test]
    fn same_or_older_is_up_to_date() {
        let body = listing(&[("v0.1.0", false, true), ("v0.2.0", false, false)]);
        assert_eq!(newest(&body, "0.2.0"), Ok(Outcome::UpToDate));
        assert_eq!(newest(&body, "0.3.0-dev"), Ok(Outcome::UpToDate));
        assert_eq!(newest("[]", "0.2.0"), Ok(Outcome::UpToDate));
    }

    #[test]
    fn a_full_release_beats_its_prerelease() {
        let body = listing(&[("v1.0.0", false, false)]);
        assert!(matches!(newest(&body, "1.0.0-rc.1"), Ok(Outcome::Newer(_))));
    }

    #[test]
    fn skips_drafts_odd_tags_and_foreign_links() {
        let mut body = listing(&[
            ("v9.0.0", true, false),
            ("nightly", false, true),
            ("v0.1.0", false, false),
        ]);
        body.pop();
        body.push_str(r#",{"tag_name":"v5.0.0","html_url":"https://evil.example/x"}]"#);
        assert_eq!(newest(&body, "0.1.0"), Ok(Outcome::UpToDate));
    }

    #[test]
    fn rejects_replies_that_arent_listings() {
        assert!(matches!(
            newest(r#"{"message":"rate limited"}"#, "0.1.0"),
            Err(Error::BadReply(_))
        ));
        assert!(matches!(newest("<html>", "0.1.0"), Err(Error::BadReply(_))));
    }

    /// Asks the real GitHub. Run by hand: `cargo test -p panna-update -- --ignored`.
    #[test]
    #[ignore = "uses the network"]
    fn live_check() {
        let result = check("0.0.1");
        println!("{result:?}");
        assert!(
            matches!(result, Ok(Outcome::Newer(_)) | Err(Error::Offline(_))),
            "{result:?}"
        );
    }
}
