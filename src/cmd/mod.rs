// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: Oliver Tale-Yazdi <oliver@tasty.limo>

//! Sub-command definition and implementation.

pub mod lint;
pub mod trace;

use cargo_metadata::{Dependency, Metadata, MetadataCommand, Package, Resolve};

/// See out how Rust dependencies and features are enabled.
#[derive(Debug, clap::Parser)]
pub struct Command {
	#[clap(subcommand)]
	subcommand: SubCommand,

	#[clap(long, global = true)]
	quiet: bool,
}

/// Sub-commands of the [Root](Command) command.
#[derive(Debug, clap::Subcommand)]
enum SubCommand {
	Trace(trace::TraceCmd),
	Lint(lint::LintCmd),
}

impl Command {
	pub fn run(&self) {
		if self.quiet {
			log::set_max_level(log::LevelFilter::Error);
		} else {
			log::set_max_level(log::LevelFilter::Info);
		}

		match &self.subcommand {
			SubCommand::Trace(cmd) => cmd.run(),
			SubCommand::Lint(cmd) => cmd.run(),
		}
	}
}

/// Arguments for how to load cargo metadata from a workspace.
#[derive(Debug, clap::Parser)]
pub struct CargoArgs {
	/// Cargo manifest path or directory.
	///
	/// For directories it appends a `Cargo.toml`.
	#[arg(long, global = true, default_value = "Cargo.toml")]
	pub manifest_path: std::path::PathBuf,

	/// Whether to only consider workspace crates.
	#[clap(long, global = true)]
	pub workspace: bool,

	/// Whether to use offline mode.
	#[clap(long, global = true)]
	pub offline: bool,
}

impl CargoArgs {
	/// Load the metadata of the rust project.
	pub fn load_metadata(&self) -> Result<Metadata, String> {
		let mut cmd = MetadataCommand::new();
		let manifest_path = if self.manifest_path.is_dir() {
			self.manifest_path.join("Cargo.toml")
		} else {
			self.manifest_path.clone()
		};
		log::debug!("Manifest at: {:?}", manifest_path);
		cmd.manifest_path(&manifest_path);
		cmd.features(cargo_metadata::CargoOpt::AllFeatures);

		if self.workspace {
			cmd.no_deps();
		}
		if self.offline {
			cmd.other_options(vec!["--offline".to_string()]);
		}

		cmd.exec().map_err(|e| format!("Failed to load metadata: {e}"))
	}
}

/// Resolve the dependency `dep` of `pkg` within the metadata.
///
/// This checks whether the dependency is a workspace or external crate and resolves it accordingly.
pub(crate) fn resolve_dep(pkg: &Package, dep: &Dependency, meta: &Metadata) -> Option<Package> {
	match meta.resolve.as_ref() {
		Some(resolve) => resolve_dep_from_graph(pkg, dep, (meta, resolve)),
		None => resolve_dep_from_workspace(dep, meta),
	}
}

/// Resolve the dependency `dep` within the workspace.
///
/// Errors if `dep` is not a workspace member.
pub(crate) fn resolve_dep_from_workspace(dep: &Dependency, meta: &Metadata) -> Option<Package> {
	for work in meta.workspace_packages() {
		if work.name == dep.name {
			return meta.packages.iter().find(|pkg| pkg.id == work.id).cloned()
		}
	}
	None
}

/// Resolve the dependency `dep` of `pkg` within the resolve graph.
///
/// The resolve graph should only be used for external crates. I did not try what happens for
/// workspace members - better don't do it.
pub(crate) fn resolve_dep_from_graph(
	pkg: &Package,
	dep: &Dependency,
	(meta, resolve): (&Metadata, &Resolve),
) -> Option<Package> {
	let dep_name = dep.name.replace('-', "_");
	let resolved_pkg = resolve.nodes.iter().find(|node| node.id == pkg.id)?;
	let resolved_dep_id = resolved_pkg.deps.iter().find(|node| node.name == dep_name)?;
	let resolve_dep = meta.packages.iter().find(|pkg| pkg.id == resolved_dep_id.pkg)?;

	Some(resolve_dep.clone())
}
