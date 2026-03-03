use std::path::PathBuf;

use crate::MigrateArgs;

pub(crate) fn execute(args: MigrateArgs) -> anyhow::Result<()> {
	let MigrateArgs { file } = args;
	migrate_file(file)
}

fn migrate_file(path: PathBuf) -> anyhow::Result<()> {
	let contents = fs_err::read_to_string(&path)?;
	let migrated = agentgateway::types::local::migrate_deprecated_local_config(&contents)?;
	fs_err::write(&path, migrated)?;
	println!("Migrated config: {}", path.display());
	Ok(())
}
