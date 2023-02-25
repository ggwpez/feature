// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: Oliver Tale-Yazdi <oliver@tasty.limo>

//! Lint your feature usage by analyzing crate metadata.

use crate::{autofix::AutoFixer, cmd::resolve_dep, prelude::*, CrateId};
use cargo_metadata::{PackageId, Package, Metadata};
use core::{
	fmt,
	fmt::{Display, Formatter},
};
use std::{
	collections::{BTreeMap, BTreeSet},
	fs::canonicalize, path::PathBuf,
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
	/// A specific feature never enables a specific other feature.
	NeverEnables(NeverEnablesCmd),
	/// A specific feature never enables a specific other feature.
	NeverImplies(NeverImpliesCmd),
	/// A specific feature is only implied by a specific set of other features.
	OnlyImplies(OnlyImpliesCmd),
	WhyEnabled(WhyEnabledCmd),
}

#[derive(Debug, clap::Parser)]
pub struct WhyEnabledCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

	#[clap(long)]
	from: Option<String>,

	#[clap(long, short)]
	to: String,

	#[clap(long)]
	feature: String,
}

#[derive(Debug, clap::Parser)]
pub struct OnlyImpliesCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

	#[clap(long, short)]
	package: String,

	#[clap(long)]
	feature: String,

	#[clap(long)]
	preconditions: Vec<String>,
}

#[derive(Debug, clap::Parser)]
pub struct NeverEnablesCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

	/// The left side of the feature implication.
	///
	/// Can be set to `default` for the default feature set.
	#[clap(long, required = true)]
	precondition: String,

	/// The right side of the feature implication.
	///
	/// If [precondition] is enabled, this stays disabled.
	#[clap(long, required = true)]
	stays_disabled: String,
}

#[derive(Debug, clap::Parser)]
pub struct NeverImpliesCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

	/// The left side of the feature implication.
	///
	/// Can be set to `default` for the default feature set.
	#[clap(long, required = true)]
	precondition: String,

	/// The right side of the feature implication.
	///
	/// If [precondition] is enabled, this stays disabled.
	#[clap(long, required = true)]
	stays_disabled: String,

	/// Show the source location of crates in the output.
	#[clap(long)]
	show_source: bool,

	/// Show the version of the crates in the output.
	#[clap(long)]
	show_version: bool,

	/// Delimiter for rendering dependency paths.
	#[clap(long, default_value = " -> ")]
	path_delimiter: String,
}

/// Verifies that rust features are properly propagated.
#[derive(Debug, clap::Parser)]
pub struct PropagateFeatureCmd {
	#[allow(missing_docs)]
	#[clap(flatten)]
	cargo_args: super::CargoArgs,

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

	#[clap(long)]
	modify_paths: Vec<PathBuf>,

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
			SubCommand::NeverEnables(cmd) => cmd.run(),
			SubCommand::NeverImplies(cmd) => cmd.run(),
			SubCommand::WhyEnabled(cmd) => cmd.run(),
			SubCommand::OnlyImplies(cmd) => cmd.run(),
		}
	}
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct CrateAndFeature(String, String);

impl Display for CrateAndFeature {
	fn fmt(&self, f: &mut Formatter) -> fmt::Result {
		write!(f, "{}/{}", self.0, self.1)
	}
}

impl NeverImpliesCmd {
	pub fn run(&self) {
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		log::info!(
			"Checking that feature {:?} never implies {:?}",
			self.precondition,
			self.stays_disabled
		);
		let pkgs = meta.packages.clone();
		let dag = build_feature_dag(&meta, &pkgs);

		for CrateAndFeature(pkg, feature) in dag.lhs_nodes() {
			let crate_and_feature = CrateAndFeature(pkg.clone(), feature.clone());
			if feature == &self.precondition {
				let Some(path) = dag.reachable_predicate(&crate_and_feature, |CrateAndFeature(_, enabled)| enabled == &self.stays_disabled) else {
					continue;
				};

				// TODO cleanup this cluster fuck
				let lookup = |id: &str| {
					pkgs
						.iter().find(|pkg| pkg.id.to_string() == id)
						.unwrap_or_else(|| panic!("Could not find crate {id} in the metadata"))
				};

				let delimiter = self.path_delimiter.replace("\\n", "\n").replace("\\t", "\t");
				let mut out = String::new();
				let mut is_first = true;

				path.for_each(|CrateAndFeature(id, feature)| {
					let krate = lookup(id);
					if !is_first {
						out.push_str(&delimiter);
					}
					is_first = false;
					out.push_str(&format!("{}/{}", krate.name, feature));
					if self.show_version {
						out.push_str(&format!(" v{}", krate.version));
					}
					if self.show_source {
						if let Some(source) = krate.source.as_ref() {
							out.push_str(&format!(" ({})", source.repr));
						}
					}
				});
				println!("Feature '{}' implies '{}' via path:\n  {}", self.precondition,
				 self.stays_disabled, out);

				std::process::exit(1);
			}
		}
	}
}

impl NeverEnablesCmd {
	pub fn run(&self) {
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		log::info!(
			"Checking that feature {:?} never enables {:?}",
			self.precondition,
			self.stays_disabled
		);
		let pkgs = meta.packages.clone();
		// Crate -> dependencies that invalidate the assumption.
		let mut offenders = BTreeMap::<CrateId, BTreeSet<CrateId>>::new();

		for pkg in pkgs.iter() {
			let Some(enabled) = pkg.features.get(&self.precondition) else {
				continue;
			};
			// TODO do the same in other command.
			if enabled.contains(&format!("{}", self.stays_disabled)) {
				offenders.entry(pkg.id.to_string()).or_default().insert(pkg.id.to_string());
			}

			for dep in pkg.dependencies.iter() {
				let Some(dep) = resolve_dep(&pkg, dep, &meta) else {
					continue;
				};

				if enabled.contains(&format!("{}/{}", dep.name, self.stays_disabled)) {
					offenders.entry(pkg.id.to_string()).or_default().insert(dep.id.to_string());
				}
			}
		}

		let lookup = |id: &str| {
			let id = PackageId { repr: id.to_string() }; // TODO optimize
			pkgs.iter()
				.find(|pkg| pkg.id == id)
				.unwrap_or_else(|| panic!("Could not find crate {id} in the metadata"))
		};

		for (id, deps) in offenders {
			let krate = lookup(&id);

			println!("crate {:?}\n  feature {:?}", krate.name, self.precondition);
			// TODO support multiple left/right side features.
			println!("    enables feature {:?} on dependencies:", self.stays_disabled);

			for dep in deps {
				println!("      {}", lookup(&dep).name);
			}
		}
	}
}

impl PropagateFeatureCmd {
	pub fn run(&self) {
		// Allowed dir that we can write to.
		let allowed_dir = canonicalize(&self.cargo_args.manifest_path).unwrap();
		let allowed_dir = allowed_dir.parent().unwrap();
		let feature = self.feature.clone();
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		let pkgs = meta.packages.iter().collect::<Vec<_>>();
		let mut to_check = pkgs.clone();
		if !self.packages.is_empty() {
			to_check =
				pkgs.iter().filter(|pkg| self.packages.contains(&pkg.name)).cloned().collect();
		}
		if to_check.is_empty() {
			panic!("No packages found: {:?}", self.packages);
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
				let Some(dep) = resolve_dep(pkg, dep, &meta) else {
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
			
			let mut fixer = if self.fix {
				if krate_path.starts_with(allowed_dir) || self.modify_paths.iter().any(|p| krate_path.starts_with(p)) {
					Some(AutoFixer::from_manifest(&krate_path).unwrap())
				} else {
					log::info!(
						"Cargo path is outside of the workspace: {:?} not in {:?}",
						krate_path.display(),
						allowed_dir.display()
					);
					None
				}
			} else {
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
			if let Some(fixer) = fixer.as_mut() {
				fixer.save().unwrap();
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

impl OnlyImpliesCmd {
	pub fn run(&self) {
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		let pkgs = meta.packages.clone();
		let dag = build_feature_dag(&meta, &pkgs);
		let mut found = false;

		let lookup = |id: &str| {
			pkgs
				.iter().find(|pkg| pkg.id.to_string() == id)
		};
		let resolved = pkgs.iter().find(|pkg| pkg.name == self.package).unwrap();
		let to = CrateAndFeature(resolved.id.to_string().clone(), self.feature.clone());
		log::info!("Looking for paths to {}/{}", to.0, to.1);

		for lhs in dag.lhs_nodes() {
			if let Some(path) = dag.any_path(&lhs, &to) {
				if !self.preconditions.contains(&lhs.1) {
					let mut out = String::new();
					let mut is_first = true;

					let delimiter = " -> ";
					path.for_each(|CrateAndFeature(id, feature)| {
						let krate = lookup(id).unwrap();
						if !is_first {
							out.push_str(&delimiter);
						}
						is_first = false;
						out.push_str(&format!("{}/{}", krate.name, feature));
					});

					println!("Found path: {out}");
					found = true;
				}
			}
		}

		if found {
			std::process::exit(1);
		}
	}
}

impl WhyEnabledCmd {
	pub fn run(&self) {
		let meta = self.cargo_args.load_metadata().expect("Loads metadata");
		let pkgs = meta.packages.clone();
		let dag = build_feature_dag(&meta, &pkgs);
		let mut found = false;

		let lookup = |id: &str| {
			pkgs
				.iter().find(|pkg| pkg.id.to_string() == id)
		};
		
		for (lhs, rhs) in dag.edges.iter() {
			for rhs in rhs.iter()  {
				// A bit ghetto, but i don't want to loose unresolved rhs crates.
				let resolved = lookup(&rhs.0).map(|r| r.name.clone()).unwrap_or(rhs.0.clone());

				if resolved == self.to && rhs.1 == self.feature {
					let lhs_resolved = lookup(&lhs.0).unwrap();
					found = true;
					println!("Feature {}/{} is enabled by: {}/{}", resolved, rhs.1, lhs_resolved.name, lhs.1);
				}
			}
		}

		if !found {
			// TODO find edit distance to the closest one
			println!("Did not find package {}", self.to);
			std::process::exit(1);
		}
	}
}

fn build_feature_dag(meta: &Metadata, pkgs: &Vec<Package>) -> Dag<CrateAndFeature> {
	let mut dag = Dag::new();
	
	for pkg in pkgs.iter() {
		for dep in &pkg.dependencies {
			if dep.uses_default_features {
				dag.add_edge(
					CrateAndFeature(pkg.id.to_string(), "default".into()),
					CrateAndFeature(dep.name.clone(), "default".into()),
				);
			}
			for feature in &dep.features {
				dag.add_edge(
					CrateAndFeature(pkg.id.to_string(), "default".into()),
					CrateAndFeature(dep.name.clone(), feature.into()),
				);
			}
		}

		for (feature, deps) in pkg.features.iter() {
			for dep in deps {
				if dep.contains(":") {
					let mut splits = dep.split(":");
					let dep = splits.nth(1).unwrap();
					let dep_feature = "default";
					//log::info!("Resolving '{}' as dependency of {}: {:?}", dep, pkg.name,
					// pkg.dependencies.iter().find(|d| d.name == dep));
					let dep = pkg
						.dependencies
						.iter()
						.find(|d| d.rename.clone().unwrap_or(d.name.clone()) == dep)
						.unwrap();

					let dep_id = match resolve_dep(pkg, dep, &meta) {
						None => {
							// This can happen for optional dependencies who are not enabled, or
							// a weird `target` is specified or it is a dev dependency.
							// In this case we just go by name. It is a dead-end anyway.
							dep.name.clone()
						},
						Some(dep) => dep.id.to_string(),
					};
					dag.add_edge(
						CrateAndFeature(pkg.id.to_string(), feature.clone()),
						CrateAndFeature(dep_id.clone(), dep_feature.into()),
					);

					//log::info!(
					//	"Adding: ({}, {}) -> ({}, {})",
					//	pkg.name,
					//	feature,
					//	dep.name,
					//	dep_feature
					//);
				} else if dep.contains("/") {
					let mut splits = dep.split("/");
					let dep = splits.next().unwrap().replace("?", "");
					let dep_feature = splits.next().unwrap();

					let dep = pkg
						.dependencies
						.iter()
						.find(|d| d.rename.clone().unwrap_or(d.name.clone()) == dep)
						.expect(&format!(
							"Could not resolve dep {} of {}",
							dep,
							pkg.id.to_string()
						));

					let dep_id = match resolve_dep(pkg, dep, &meta) {
						None => {
							// This can happen for optional dependencies who are not enabled, or
							// a weird `target` is specified or it is a dev dependency.
							// In this case we just go by name. It is a dead-end anyway.
							dep.name.clone()
						},
						Some(dep) => dep.id.to_string(),
					};
					dag.add_edge(
						CrateAndFeature(pkg.id.to_string(), feature.clone()),
						CrateAndFeature(dep_id.clone(), dep_feature.into()),
					);

					//log::info!(
					//	"Adding: ({}, {}) -> ({}, {})",
					//	pkg.name,
					//	feature,
					//	dep.name,
					//	dep_feature
					//);
				} else {
					let dep_feature = dep;
					// Sanity check
					assert!(pkg.features.contains_key(dep_feature));
					// Enables one of its own features.
					//log::info!(
					//	"Adding: ({}, {}) -> ({}, {})",
					//	pkg.name,
					//	feature,
					//	pkg.name,
					//	dep_feature
					//);
					dag.add_edge(
						CrateAndFeature(pkg.id.to_string(), feature.clone()),
						CrateAndFeature(pkg.id.to_string(), dep_feature.into()),
					)
				}
			}
		}
	}
	dag
}
