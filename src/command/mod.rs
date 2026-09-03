use anyhow::{Context, Result};

use std::process::{Child, ChildStdout, Command as StdCommand, Stdio};

pub struct Command {
    pub cmd: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    stdin: Option<ChildStdout>,
}

#[allow(unused)]
impl Command {
    pub fn new(name: &str) -> Self {
        Self {
            cmd: name.to_string(),
            args: vec![],
            env: vec![],
            stdin: None,
        }
    }

    pub fn add_args<T>(mut self, args: impl IntoIterator<Item = T>) -> Self
    where
        T: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn add_env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn pipe(mut self, mut parent: Command) -> Result<Self> {
        let mut parent_cmd = parent.build_cmd();
        parent_cmd.stdout(Stdio::piped());

        let mut child = parent_cmd
            .spawn()
            .with_context(|| format!("failed to spawn parent process: {}", parent.cmd))?;
        let stdout = child
            .stdout
            .take()
            .with_context(|| format!("failed to get stdout from parent process: {}", parent.cmd))?;

        self.stdin = Some(stdout);
        Ok(self)
    }

    pub fn spawn(&mut self) -> Result<Child> {
        let mut cmd = self.build_cmd();
        Ok(cmd.spawn()?)
    }

    fn build_cmd(&mut self) -> StdCommand {
        let mut command = std::process::Command::new(&self.cmd);

        command
            .args(&self.args)
            .envs(self.env.clone())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if let Some(stdin) = self.stdin.take() {
            command.stdin(stdin);
        }

        command
    }
}
