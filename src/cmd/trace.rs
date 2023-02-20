// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: Oliver Tale-Yazdi <oliver@tasty.limo>

//! Trace the dependency path from one crate to another.

use super::*;
use crate::{dag::Dag, CrateId};
use cargo_metadata::{Metadata, Package};
use clap::Parser;
use std::collections::{BTreeMap, BTreeSet};

/// Trace the dependency path from one crate to another.
#[derive(Debug, Parser)]
pub struct TraceCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

	/// Show the source location of crates in the output.
	#[clap(long)]
	show_source: bool,

	/// Show the version of the crates in the output.
	#[clap(long)]
	show_version: bool,

	/// Delimiter for rendering dependency paths.
	#[clap(long, default_value = " -> ")]
	path_delimiter: String,

	/// Do not unify versions but treat `(id, version)` as a unique crate in the dependency graph.
	///
	/// Unifying the versions would mean that they are factored out and only `id` is used to
	/// identify a crate.
	#[clap(long)]
	unique_versions: bool,

	/// The root crate to start from.
	#[clap(index(1))]
	from: String,

	/// The dependency crate to end at.
	#[clap(index(2))]
	to: String,
}

impl TraceCmd {
	pub(crate) fn run(&self) {
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		let (dag, index) = self.build_dag(meta).expect("Builds dependency graph");
		let lookup = |id: &str| {
			index
				.get(id)
				.unwrap_or_else(|| panic!("Could not find crate {id} in the metadata"))
		};

		let froms = index
			.iter()
			.filter(|(_id, krate)| krate.name == self.from)
			.map(|(id, _)| id)
			.collect::<Vec<_>>();
		if froms.is_empty() {
			panic!("Could not find crate {} in the left dependency graph", self.from);
		}

		let tos = index
			.iter()
			.filter(|(_id, krate)| krate.name == self.to)
			.map(|(id, _)| id)
			.collect::<Vec<_>>();
		if tos.is_empty() {
			panic!("Could not find crate {} in the right dependency graph", self.to);
		}

		log::info!(
			"No version or features specified: Checking all {} possibly distinct paths",
			froms.len() * tos.len()
		);
		let mut paths = BTreeSet::new();

		for from in froms.iter() {
			for to in tos.iter() {
				if let Some(path) = dag.any_path(from, to) {
					paths.insert(path);
				}
			}
		}
		if paths.is_empty() {
			panic!("No path found");
		}
		log::info!("Found {} distinct paths", paths.len());
		// Unescape the delimiter.
		let delimiter = self.path_delimiter.replace("\\n", "\n").replace("\\t", "\t");

		for path in paths {
			let mut out = String::new();
			let mut is_first = true;

			path.for_each(|id| {
				let krate = lookup(id);
				if !is_first {
					out.push_str(&delimiter);
				}
				is_first = false;
				out.push_str(&krate.name);
				if self.show_version {
					out.push_str(&format!(" v{}", krate.version));
				}
				if self.show_source {
					if let Some(source) = krate.source.as_ref() {
						out.push_str(&format!(" ({})", source.repr));
					}
				}
			});

			println!("{out}");
		}
	}

	/// Build a dependency graph over the crates ids and return an index of all crates.
	fn build_dag(
		&self,
		meta: Metadata,
	) -> Result<(Dag<CrateId>, BTreeMap<CrateId, Package>), String> {
		let mut dag = Dag::new();
		let mut index = BTreeMap::new();

		for pkg in meta.packages.clone().into_iter() {
			let id = pkg.id.to_string();
			dag.add_node(id.clone());
			index.insert(pkg.id.to_string(), pkg.clone());

			for dep in pkg.dependencies.iter() {
				if let Some(dep) = resolve_dep(&pkg, dep, &meta) {
					let did = dep.id.to_string();
					dag.add_edge(id.clone(), did);
				}
			}
		}

		Ok((dag, index))
	}
}
