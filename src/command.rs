use anyhow::{Context, Result};

use std::{
    ffi::OsStr,
    process::{Child, Command as StdCommand, Stdio},
};

pub struct Command(StdCommand);

impl Command {
    pub fn new(name: &str) -> Self {
        let mut command = StdCommand::new(name);

        command.stdout(Stdio::null()).stderr(Stdio::null());

        Self(command)
    }

    pub fn add_args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.0.args(args);

        self
    }

    pub fn add_env(mut self, k: &str, v: &str) -> Self {
        self.0.env(k, v);

        self
    }

    pub fn pipe(mut self, mut parent: Command) -> Result<Self> {
        parent.0.stdout(Stdio::piped());

        let mut child = parent.0.spawn().with_context(|| {
            format!(
                "failed to spawn parent process: {:?}",
                parent.0.get_program()
            )
        })?;
        let stdout = child.stdout.take().context("failed to get parent stdout")?;

        self.0.stdin(stdout);

        Ok(self)
    }

    pub fn spawn(&mut self) -> Result<Child> {
        Ok(self.0.spawn()?)
    }
}
