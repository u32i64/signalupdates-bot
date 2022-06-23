use anyhow::{anyhow, Context};
use serde_json::json;
use worker::{console_log, Method, Url};

use crate::{
    platform::Platform::{self},
    types, utils,
};

const DISCOURSE_API_POSTING_URL: &str = "https://community.signalusers.org/posts.json";

#[derive(Debug)]
pub struct Post {
    platform: Platform,
    previous_tag: String,
    new_tag: String,
    commits: Vec<Commit>,
}

impl Post {
    pub fn new(
        platform: Platform,
        previous_tag: impl Into<String>,
        new_tag: impl Into<String>,
        commits: Vec<Commit>,
    ) -> Self {
        Self {
            platform,
            previous_tag: previous_tag.into(),
            new_tag: new_tag.into(),
            commits,
        }
    }

    pub fn markdown_text(&self) -> String {
        let commits = self
            .commits
            .iter()
            .enumerate()
            .map(|(index, commit)| commit.markdown_text(index))
            .collect::<Vec<_>>()
            .join("\n");

        let previous_version = self.previous_tag.replace('v', "");
        let new_version = self.new_tag.replace('v', "");

        let platform = self.platform;
        let availability_notice = platform.availability_notice();
        let comparison_url = platform.github_comparison_url(&self.previous_tag, &self.new_tag);

        let commits_count = self.commits.len();
        let (commits_prefix, commits_postfix) = match commits_count {
            0..=20 => ("", ""),
            _ => ("[details=\"Show commits\"]\n\n", "\n\n[/details]"),
        };

        let commits_word_suffix = if commits_count == 1 { "" } else { "s" };

        format!(
            "## :tada: - New Version: {new_version}{availability_notice}

[quote]
{commits_count} new commit{commits_word_suffix} since {previous_version}:

{commits_prefix}{commits}{commits_postfix}

---
Gathered from [signalapp/Signal-{platform}]({comparison_url})
[/quote]"
        )
    }

    pub async fn post(
        &self,
        api_key: String,
        topic_id: u64,
        reply_to_post_number: Option<u64>,
    ) -> anyhow::Result<u64> {
        let markdown_text = self.markdown_text();
        console_log!("posting post with markdown_text = {:?}", markdown_text);

        let body = json!({
            "topic_id": topic_id,
            "reply_to_post_number": reply_to_post_number,
            "raw": markdown_text,
        });

        let url = Url::parse(DISCOURSE_API_POSTING_URL).context("could not parse URL")?;
        let request = utils::create_request(url, Method::Post, Some(body), Some(api_key))?;

        let api_response: types::discourse::PostApiResponse =
            utils::get_json_from_request(request).await?;

        console_log!("api_response = {:?}", api_response);

        match api_response.post_number {
            Some(number) => Ok(number),
            None => Err(anyhow!(
                "discourse API response did not include the post number, posting likely failed"
            )),
        }
    }
}

#[derive(Debug)]
pub struct Commit {
    platform: Platform,
    message_lines: Vec<String>,
    sha: String,
}

impl Commit {
    pub fn new(
        platform: Platform,
        full_message: impl Into<String>,
        sha: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            message_lines: full_message
                .into()
                .split('\n')
                .filter(|line| {
                    !line.contains("Co-Authored-By") && !line.contains("This reverts commit")
                })
                .map(String::from)
                .collect(),
            sha: sha.into(),
        }
    }

    pub fn markdown_text(&self, index: usize) -> String {
        let index = index + 1;

        let message = match self.message_lines.get(0) {
            Some(line) => line,
            None => "*Empty commit message*",
        };

        let commit_url = self.platform.github_commit_url(&self.sha);

        let main_content = format!("- {message} [[{index}]]({commit_url})\n");

        let details = match self.message_lines.len() {
            2.. => format!("\n    {}", self.message_lines[1..].join("\n    ")),
            _ => String::from(""),
        };

        main_content + &details
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;
    use Platform::*;

    #[test_case(Android, "Test commit.", "abcdef" => "- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)\n".to_string(); "one line")]
    #[test_case(Android, "Test commit.\nAnother line.", "abcdef" => "- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)\n\n    Another line.".to_string(); "two lines")]
    #[test_case(Android, "Test commit.\nAnother line.\nAnd another line.", "abcdef" => "- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)\n\n    Another line.\n    And another line.".to_string(); "three lines")]
    #[test_case(Android, "Test commit.\nCo-Authored-By: user", "abcdef" => "- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)\n".to_string(); "Co-Authored-By is removed")]
    #[test_case(Android, "Revert \"Test commit\".\nThis reverts commit fedcba.", "abcdef" => "- Revert \"Test commit\". [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)\n".to_string(); "This reverts commit is removed")]
    fn commit_markdown(
        platform: Platform,
        full_message: impl Into<String>,
        sha: impl Into<String>,
    ) -> String {
        Commit::new(platform, full_message, sha).markdown_text(0)
    }

    #[test_case(Android, "v1.2.3", "v1.2.4", vec![
        Commit::new(Android, "Test commit.", "abcdef")
    ] => "## :tada: - New Version: 1.2.4
(Not Yet) Available via [Firebase App Distribution](https://community.signalusers.org/t/17538)

[quote]
1 new commit since 1.2.3:

- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)


---
Gathered from [signalapp/Signal-Android](https://github.com/signalapp/Signal-Android/compare/v1.2.3...v1.2.4)
[/quote]".to_string(); "one commit")]
    #[test_case(Android, "v1.2.3", "v1.2.4", vec![
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Bump version to 1.2.4", "abc123")
    ] => "## :tada: - New Version: 1.2.4
(Not Yet) Available via [Firebase App Distribution](https://community.signalusers.org/t/17538)

[quote]
2 new commits since 1.2.3:

- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Bump version to 1.2.4 [[2]](https://github.com/signalapp/Signal-Android/commit/abc123)


---
Gathered from [signalapp/Signal-Android](https://github.com/signalapp/Signal-Android/compare/v1.2.3...v1.2.4)
[/quote]".to_string(); "two commits")]
    #[test_case(Android, "v1.2.3", "v1.2.4", vec![
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),

        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),

        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),

        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),
        Commit::new(Android, "Test commit.", "abcdef"),

        Commit::new(Android, "Bump version to 1.2.4", "abc123")
    ] => "## :tada: - New Version: 1.2.4
(Not Yet) Available via [Firebase App Distribution](https://community.signalusers.org/t/17538)

[quote]
21 new commits since 1.2.3:

[details=\"Show commits\"]

- Test commit. [[1]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[2]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[3]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[4]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[5]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[6]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[7]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[8]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[9]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[10]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[11]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[12]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[13]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[14]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[15]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[16]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[17]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[18]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[19]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Test commit. [[20]](https://github.com/signalapp/Signal-Android/commit/abcdef)

- Bump version to 1.2.4 [[21]](https://github.com/signalapp/Signal-Android/commit/abc123)


[/details]

---
Gathered from [signalapp/Signal-Android](https://github.com/signalapp/Signal-Android/compare/v1.2.3...v1.2.4)
[/quote]".to_string(); "twenty one commits")]
    fn post_markdown(
        platform: Platform,
        previous_tag: impl Into<String>,
        new_tag: impl Into<String>,
        commits: Vec<Commit>,
    ) -> String {
        Post::new(platform, previous_tag, new_tag, commits).markdown_text()
    }
}
