// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: Oliver Tale-Yazdi <oliver@tasty.limo>

//! Lint your feature usage by analyzing crate metadata.

use crate::{autofix::AutoFixer, cmd::resolve_dep, CrateId};
use cargo_metadata::PackageId;
use std::{
	collections::{BTreeMap, BTreeSet},
	fs::canonicalize,
};

/// Lint your feature usage by analyzing crate metadata.
#[derive(Debug, clap::Parser)]
pub struct LintCmd {
	#[clap(subcommand)]
	subcommand: SubCommand,
}

/// Sub-commands of the [Lint](LintCmd) command.
#[derive(Debug, clap::Subcommand)]
pub enum SubCommand {
	/// Check whether features are properly propagated.
	PropagateFeature(PropagateFeatureCmd),
}

/// Verifies that rust features are properly propagated.
#[derive(Debug, clap::Parser)]
pub struct PropagateFeatureCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	tree_args: super::TreeArgs,

	/// The feature to check.
	#[clap(long, required = true)]
	feature: String,

	/// The packages to check. If empty, all packages are checked.
	#[clap(long, short, num_args(0..))]
	packages: Vec<String>,

	/// Show crate versions in the output.
	#[clap(long)]
	crate_versions: bool,

	/// Try to automatically fix the problems.
	#[clap(long)]
	fix: bool,

	/// Fix only issues with this package as dependency.
	#[clap(long)]
	fix_dependency: Option<String>,

	/// Fix only issues with this package as feature source.
	#[clap(long)]
	fix_package: Option<String>,
}

impl LintCmd {
	pub(crate) fn run(&self) {
		match &self.subcommand {
			SubCommand::PropagateFeature(cmd) => cmd.run(),
		}
	}
}

impl PropagateFeatureCmd {
	pub fn run(&self) {
		log::info!("Using manifest: {:?}", self.tree_args.manifest_path);
		// Allowed dir that we can write to.
		let allowed_dir = canonicalize(&self.tree_args.manifest_path).unwrap();
		let allowed_dir = allowed_dir.parent().unwrap();
		let feature = self.feature.clone();
		let meta = self.tree_args.load_metadata().expect("Loads metadata");
		let pkgs = meta.packages.iter().collect::<Vec<_>>();
		let mut to_check = pkgs.clone();
		if !self.packages.is_empty() {
			to_check =
				pkgs.iter().filter(|pkg| self.packages.contains(&pkg.name)).cloned().collect();
		}
		if to_check.is_empty() {
			panic!("No packages found: {:?}", self.packages);
		}

		if let Some(root) = meta.root_package() {
			println!("Analyzing {root:?}");
		} else {
			println!("Analyzing workspace");
		}
		let lookup = |id: &str| {
			let id = PackageId { repr: id.to_string() }; // TODO optimize
			pkgs.iter()
				.find(|pkg| pkg.id == id)
				.unwrap_or_else(|| panic!("Could not find crate {id} in the metadata"))
		};

		// (Crate that is not forwarding the feature) -> (Dependency that it is not forwarded to)
		let mut propagate_missing = BTreeMap::<CrateId, BTreeSet<CrateId>>::new();
		// (Crate that missing the feature) -> (Dependency that has it)
		let mut feature_missing = BTreeMap::<CrateId, BTreeSet<CrateId>>::new();
		// Crate that has the feature but does not need it.
		let mut feature_maybe_unused = BTreeSet::<CrateId>::new();

		for pkg in to_check.iter() {
			let mut feature_used = false;
			// TODO that it does not enable other features.

			for dep in pkg.dependencies.iter() {
				// TODO handle default features.
				// Resolve the dep according to the metadata.
				let resolved = resolve_dep(pkg, dep, &meta);

				let Some(dep) = resolved else {
					// Either outside workspace or not resolved, possibly due to not being used at all because of the target or whatever.
					feature_used = true;
					continue;
				};

				if dep.features.contains_key(&feature) {
					match pkg.features.get(&feature) {
						None => {
							feature_missing
								.entry(pkg.id.to_string())
								.or_default()
								.insert(dep.id.to_string());
						},
						Some(enabled) => {
							if !enabled.contains(&format!("{}/{}", dep.name, feature)) {
								propagate_missing
									.entry(pkg.id.to_string())
									.or_default()
									.insert(dep.id.to_string());
							} else {
								// All ok
								feature_used = true;
							}
						},
					}
				}
			}

			if !feature_used && pkg.features.contains_key(&feature) {
				feature_maybe_unused.insert(pkg.id.to_string());
			}
		}
		let faulty_crates: BTreeSet<CrateId> = propagate_missing
			.keys()
			.chain(feature_missing.keys())
			.chain(feature_maybe_unused.iter())
			.cloned()
			.collect();

		let (mut errors, warnings) = (0, 0);
		let mut fixes = 0;
		for krate in faulty_crates {
			let krate = lookup(&krate);
			// check if we can modify in allowed_dir
			let krate_path = canonicalize(krate.manifest_path.clone().into_std_path_buf()).unwrap();
			let mut fixer = if krate_path.starts_with(allowed_dir) {
				Some(AutoFixer::from_manifest(&krate_path).unwrap())
			} else {
				log::info!(
					"Cannot fix {} because it is not in the allowed directory {}",
					krate.name,
					allowed_dir.display()
				);
				None
			};
			println!("crate {:?}\n  feature {:?}", krate.name, feature);

			// join
			if let Some(deps) = feature_missing.get(&krate.id.to_string()) {
				let joined = deps
					.iter()
					.map(|d| lookup(d))
					.map(|dep| dep.name.to_string())
					.collect::<Vec<_>>()
					.join("\n      ");
				println!(
					"    must exit because {} dependencies have it:\n      {}",
					deps.len(),
					joined
				);
				errors += 1;
			}
			if let Some(deps) = propagate_missing.get(&krate.id.to_string()) {
				let joined = deps
					.iter()
					.map(|d| lookup(d))
					.map(|dep| dep.name.to_string())
					.collect::<Vec<_>>()
					.join("\n      ");
				println!("    must propagate to:\n      {joined}");

				if self.fix && self.fix_package.as_ref().map_or(true, |p| p == &krate.name) {
					for dep in deps {
						let dep_name = &lookup(dep).name;
						if self.fix_dependency.as_ref().map_or(true, |d| d == dep_name) {
							if let Some(fixer) = fixer.as_mut() {
								fixer
									.add_to_feature(
										&feature,
										format!("{dep_name}/{feature}").as_str(),
									)
									.unwrap();
								log::warn!(
									"Added feature {feature} to {dep_name} in {}",
									krate.name
								);
								fixes += 1;
							}
						}
					}
				}
				errors += 1;
			}
			if fixes > 0 {
				fixer.unwrap().save().unwrap();
			}
			//if let Some(_dep) = feature_maybe_unused.get(&krate.id.to_string()) {
			//	if !feature_missing.contains_key(&krate.id.to_string()) &&
			// !propagate_missing.contains_key(&krate.id.to_string()) 	{
			//		println!("    is not used by any dependencies");
			//		warnings += 1;
			//	}
			//}
		}
		if errors > 0 || warnings > 0 {
			println!("Generated {errors} errors and {warnings} warnings and fixed {fixes} issues.");
		}
	}
}
