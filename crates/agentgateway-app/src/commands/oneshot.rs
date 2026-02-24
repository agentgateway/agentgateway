use std::fs::OpenOptions;
use std::process::{Child, Command, Stdio};

use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;

use anyhow::Context;

use crate::{ConfigArgs, OneshotArgs};

struct SpawnedSubprocess {
	child: Child,
	ready_reader: File,
}

pub(crate) fn execute(args: OneshotArgs) -> anyhow::Result<()> {
	let mut subprocess = spawn_subprocess(&args)?;
	if let Err(err) = wait_for_subprocess_readiness(&mut subprocess) {
		terminate_subprocess(&mut subprocess.child);
		return Err(err);
	}

	exec_target_command(args.command, subprocess.child)
}

#[cfg(unix)]
fn spawn_subprocess(args: &OneshotArgs) -> anyhow::Result<SpawnedSubprocess> {
	let (read_fd, write_fd) = create_ready_pipe()?;

	let mut command = Command::new(std::env::current_exe()?);
	append_config_args(&mut command, &args.config);
	command.env("READY_FD", "3");
	configure_subprocess_output(&mut command, args.agentgateway_output.as_ref())?;
	unsafe {
		command.pre_exec(move || {
			// Terminate when parent terminates...
			if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
				return Err(std::io::Error::last_os_error());
			}

			// Duplicate `write_fd` to fd 3 in the child
			if libc::dup2(write_fd.as_raw_fd(), 3) == -1 {
				return Err(std::io::Error::last_os_error());
			}
			// Close the original fd if it's not already 3
			if write_fd.as_raw_fd() != 3 {
				libc::close(write_fd.as_raw_fd());
			}
			Ok(())
		});
	}

	let child = command
		.spawn()
		.context("failed to spawn agentgateway subprocess")?;

	Ok(SpawnedSubprocess {
		child,
		ready_reader: read_fd.into(),
	})
}

#[cfg(unix)]
fn create_ready_pipe() -> anyhow::Result<(OwnedFd, OwnedFd)> {
	let mut fds = [-1, -1];
	unsafe {
		if libc::pipe(fds.as_mut_ptr()) == -1 {
			return Err(std::io::Error::last_os_error())
				.context("failed to create oneshot readiness signal pipe");
		}
		Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])))
	}
}

#[cfg(not(unix))]
fn spawn_subprocess(_args: &OneshotArgs) -> anyhow::Result<SpawnedSubprocess> {
	anyhow::bail!("oneshot is only supported on unix targets")
}

fn append_config_args(command: &mut Command, config: &ConfigArgs) {
	if let Some(config) = &config.config {
		command.arg("--config").arg(config);
	}
	if let Some(file) = &config.file {
		command.arg("--file").arg(file);
	}
}

fn configure_subprocess_output(
	command: &mut Command,
	output_path: Option<&std::path::PathBuf>,
) -> anyhow::Result<()> {
	if let Some(output_path) = output_path {
		let output = OpenOptions::new()
			.create(true)
			.append(true)
			.open(output_path)
			.with_context(|| {
				format!(
					"failed to open agentgateway output file: {}",
					output_path.display()
				)
			})?;
		let output_err = output
			.try_clone()
			.context("failed to duplicate agentgateway output file handle")?;
		command.stdout(Stdio::from(output));
		command.stderr(Stdio::from(output_err));
	} else {
		command.stdout(Stdio::inherit());
		command.stderr(Stdio::inherit());
	}
	Ok(())
}

fn wait_for_subprocess_readiness(subprocess: &mut SpawnedSubprocess) -> anyhow::Result<()> {
	// block until child closes its end
	let mut buf = [0u8; 1];
	let _ = subprocess.ready_reader.read(&mut buf)?; // EOF returns Ok(0)
	if let Some(status) = subprocess.child.try_wait()? {
		anyhow::bail!(
			"agentgateway subprocess terminated: {}",
			status.code().unwrap_or(-1)
		);
	}
	Ok(())
}

fn exec_target_command(command: Vec<std::ffi::OsString>, mut child: Child) -> anyhow::Result<()> {
	let mut command_iter = command.into_iter();
	let Some(program) = command_iter.next() else {
		anyhow::bail!("oneshot requires a command after '--'");
	};

	let mut target = Command::new(program);
	target
		.args(command_iter)
		.stdin(Stdio::inherit())
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit());

	let exec_error = target.exec();
	terminate_subprocess(&mut child);
	Err(anyhow::Error::new(exec_error).context("failed to exec oneshot target command"))
}

fn terminate_subprocess(child: &mut Child) {
	if child.try_wait().ok().flatten().is_some() {
		return;
	}
	let _ = child.kill();
	let _ = child.wait();
}
