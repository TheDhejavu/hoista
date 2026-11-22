//! Reaching a server Hoista made, over SSH, with the key it keeps for it.

use crate::{ports::catalog::EngineError, servers::ServerAccess};
use std::time::Duration;

/// Where a server answers.
#[derive(Debug, Clone, Copy)]
pub struct ServerAddress<'a> {
    /// Its IP address or host name.
    pub host: &'a str,
    /// Its SSH port.
    pub port: u16,
}

/// One command to run.
#[derive(Debug, Clone, Copy)]
pub struct CommandRun<'a> {
    /// What runs, as one line for the server's shell.
    pub command: &'a str,
    /// What the command reads on its standard input.
    pub input: Option<&'a [u8]>,
    /// How long it may take before Hoista stops it.
    pub timeout: Duration,
    /// How much of its output, each of standard output and standard error,
    /// is kept; the rest is cut.
    pub max_output: usize,
}

/// How a command went.
#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    /// Its exit code; None when it was stopped or the server didn't say.
    pub exit_code: Option<u32>,
    /// What it printed, up to the limit asked for.
    pub stdout: Vec<u8>,
    /// What it printed as errors, up to the limit asked for.
    pub stderr: Vec<u8>,
    /// Some output was cut at the limit.
    pub truncated: bool,
    /// It ran past its time and was stopped.
    pub timed_out: bool,
}

/// Runs commands on servers over SSH.
#[async_trait::async_trait]
pub trait RemoteServer: Send + Sync {
    /// Connects as `access` says, checks the server's host key against the
    /// one Hoista keeps, and runs one command.
    ///
    /// # Errors
    /// Reports a server that can't be reached, a host key that isn't the one
    /// kept (code `server.host_key_changed`), or a refused sign-in.
    async fn run(
        &self,
        address: ServerAddress<'_>,
        access: &ServerAccess,
        run: CommandRun<'_>,
    ) -> Result<CommandOutput, EngineError>;
}
