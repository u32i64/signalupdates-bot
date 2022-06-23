use semver::Version;
use std::fmt;
use Platform::*;

#[derive(Debug, Clone, Copy, strum_macros::EnumIter)]
pub enum Platform {
    Android,
    Desktop,
}

impl Platform {
    pub const fn github_api_tags_url(&self) -> &'static str {
        match self {
            Android => "https://api.github.com/repos/signalapp/Signal-Android/tags",
            Desktop => "https://api.github.com/repos/signalapp/Signal-Desktop/tags",
        }
    }

    pub fn should_post_version(&self, version: &Version) -> bool {
        match self {
            Android => version.build.is_empty(), // versions like 1.2.3.4 are filtered out (the "4" is parsed into `build` by lenient_semver)
            Desktop => version.pre.contains("beta"),
        }
    }

    pub fn github_api_comparison_url(&self, old: &str, new: &str) -> String {
        format!("https://api.github.com/repos/signalapp/Signal-{self}/compare/{old}...{new}")
    }

    pub fn github_comparison_url(&self, old: &str, new: &str) -> String {
        format!("https://github.com/signalapp/Signal-{self}/compare/{old}...{new}")
    }

    pub fn github_commit_url(&self, sha: &str) -> String {
        format!("https://github.com/signalapp/Signal-{self}/commit/{sha}")
    }

    pub const fn availability_notice(&self) -> &'static str {
        match self {
            Android => "\n(Not Yet) Available via [Firebase App Distribution](https://community.signalusers.org/t/17538)",
            Desktop => "",
        }
    }

    pub fn discourse_topic_slug_url(&self, version: &Version) -> String {
        format!(
            "https://community.signalusers.org/t/beta-feedback-for-the-upcoming-{}-{}-{}-release.json",
            self.to_string().to_ascii_lowercase(), version.major, version.minor
        )
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Android => "Android",
                Desktop => "Desktop",
            }
        )
    }
}
