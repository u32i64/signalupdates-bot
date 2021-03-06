use anyhow::{anyhow, bail, Context};
use semver::Version;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use worker::{console_log, console_warn, Env};
use worker_kv::KvStore;

use crate::{
    localization::{Completeness, UnsortedChanges},
    platform::Platform::{self, *},
    types::github::Tag,
};

const STATE_KV_BINDING: &str = "STATE";
const STATE_KV_KEY: &str = "state";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub android: PlatformState,
    pub ios: PlatformState,
    pub desktop: PlatformState,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PlatformState {
    pub last_posted_tag_previous_release: Tag,
    pub last_posted_tag: Tag,

    #[serde(default)]
    pub last_post_number: Option<u64>,

    #[serde(default)]
    pub posted_archiving_message: bool,

    #[serde(default)]
    pub localization_changes_completeness: Completeness,
    #[serde(default)]
    pub localization_changes: UnsortedChanges,
}

pub struct StateController {
    kv_store: KvStore,
    state: State,
}

impl StateController {
    pub async fn from_kv(env: &Env) -> anyhow::Result<Self> {
        let kv_store = env
            .kv(STATE_KV_BINDING)
            .map_err(|e| anyhow!(e.to_string()))
            .context("could not get KV store")?;

        let state: Option<State> = kv_store
            .get(STATE_KV_KEY)
            .json()
            .await
            .map_err(|e| anyhow!(e.to_string()))
            .with_context(|| format!("could not get value for key {STATE_KV_KEY}"))?;

        match state {
            Some(state) => {
                let controller = Self { kv_store, state };
                controller.log_state("loaded state from KV");
                controller.validate_state().context("invalid state")?;
                console_log!("state appears to be valid");

                Ok(controller)
            }
            None => bail!("no state in KV"),
        }
    }

    fn validate_state(&self) -> anyhow::Result<()> {
        for platform in Platform::iter() {
            let state = self.platform_state(platform);

            let last_posted_version_previous_release: Version = state
                .last_posted_tag_previous_release
                .to_version()
                .context("couldn't convert last_posted_tag_previous_release to a Version")?;

            let last_posted_version: Version = state
                .last_posted_tag
                .to_version()
                .context("couldn't convert last_posted_tag to a Version")?;

            if last_posted_version_previous_release >= last_posted_version {
                bail!("last_posted_version_previous_release >= last_posted_version for {platform}");
            }
        }

        Ok(())
    }

    pub fn platform_state(&self, platform: Platform) -> &PlatformState {
        match platform {
            Android => &self.state.android,
            Ios => &self.state.ios,
            Desktop => &self.state.desktop,
        }
    }

    fn platform_state_mut(&mut self, platform: Platform) -> &mut PlatformState {
        match platform {
            Android => &mut self.state.android,
            Ios => &mut self.state.ios,
            Desktop => &mut self.state.desktop,
        }
    }

    pub async fn set_platform_state(
        &mut self,
        platform: Platform,
        state: PlatformState,
    ) -> anyhow::Result<()> {
        let platform_state = self.platform_state_mut(platform);

        if *platform_state != state {
            *platform_state = state;
            console_log!("changed platform_state({platform}) = {platform_state:?}");

            match self.commit_changes().await {
                Ok(_) => console_log!("saved state to KV"),
                Err(e) => return Err(e.context("could not save state to KV")),
            }
        } else {
            console_warn!("platform_state({platform}) did not change");
        }

        Ok(())
    }

    async fn commit_changes(&mut self) -> anyhow::Result<()> {
        self.kv_store
            .put(STATE_KV_KEY, &self.state)
            .map_err(|e| anyhow!(e.to_string()))
            .context("could not create request to put to KV")?
            .execute()
            .await
            .map_err(|e| anyhow!(e.to_string()))
            .context("could not put to KV")
    }

    fn log_state(&self, message: &str) {
        console_log!("{message}:");

        for platform in Platform::iter() {
            console_log!(
                "^^^^^ platform_state({platform}) = {:?}",
                self.platform_state(platform)
            );
        }
    }
}
