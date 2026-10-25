use crate::command::Command;
use anyhow::{Context, Result};
use configfs::{Config, ConfigDirectory};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct KeyCommand {
    app: String,
    #[serde(default)]
    pub(crate) times: ListenTimes,
    #[serde(default)]
    pub(crate) links_with: Vec<char>,
    args: Option<Vec<String>>,
    env: Option<Vec<(String, String)>>,

    pipe: Option<Box<KeyCommand>>,
}

impl Default for KeyCommand {
    fn default() -> Self {
        Self {
            app: "".to_string(),
            times: ListenTimes::default(),
            links_with: Vec::new(),
            args: None,
            env: None,
            pipe: None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Unlimited {
    Infinite,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(untagged)]
pub(crate) enum ListenTimes {
    Limited(std::num::NonZeroUsize),
    Unlimited(Unlimited),
}

impl Default for ListenTimes {
    fn default() -> Self {
        Self::Limited(std::num::NonZeroUsize::new(1).unwrap())
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct KeyConfig {
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: f64,
    pub(crate) keys: HashMap<char, KeyCommand>,
}

fn default_timeout_seconds() -> f64 {
    1.0
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            keys: HashMap::new(),
        }
    }
}

impl KeyConfig {
    pub(crate) fn read() -> Result<Self> {
        let config: Config<Self> = Config::new(ConfigDirectory::System("keyrun"))?;
        let config = config.read()?;

        for command in config.keys.values() {
            for key in &command.links_with {
                anyhow::ensure!(
                    config.keys.contains_key(key),
                    "Missing linked binding {key:?}"
                );
            }
        }

        Ok(config)
    }

    pub(crate) fn timeout(&self) -> Result<Duration> {
        let timeout = Duration::try_from_secs_f64(self.timeout_seconds)
            .context("timeout_seconds must be a finite, positive number")?;

        anyhow::ensure!(
            !timeout.is_zero(),
            "timeout_seconds must be greater than zero"
        );
        Instant::now()
            .checked_add(timeout)
            .context("timeout_seconds is too large")?;

        Ok(timeout)
    }
}

impl KeyCommand {
    pub fn build(&self) -> Result<Command> {
        let mut cmd = Command::new(&self.app);

        if let Some(args) = &self.args {
            cmd = cmd.add_args(args);
        }

        if let Some(env) = &self.env {
            for (k, v) in env {
                cmd = cmd.add_env(k, v);
            }
        }

        if let Some(pipe_cmd) = &self.pipe {
            let mut parent = pipe_cmd.build()?;
            parent = parent.pipe(cmd)?;
            Ok(parent)
        } else {
            Ok(cmd)
        }
    }
}
