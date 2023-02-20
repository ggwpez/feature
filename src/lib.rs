// SPDX-License-Identifier: GPL-3.0-only
// SPDX-FileCopyrightText: Oliver Tale-Yazdi <oliver@tasty.limo>

//! Understand why and how features are enabled in a rust workspace. `Feature` is able to
//! automatically fix missing feature propagation to dependencies. Eventually it will be ready for
//! CI use to check an MR for consistent feature usage.
//!
//! ## Install
//!
//! ```bash
//! cargo install -f feature
//! ```
//!
//! ## Example - Fixing feature propagation
//!
//! Let's check that the `runtime-benchmarks` feature is properly passed down to all the
//! dependencies of the `frame-support` crate in the workspace of [Substrate](https://github.com/paritytech/substrate):
//!
//! ```bash
//! feature lint propagate-feature --manifest-path ../substrate/Cargo.toml --feature runtime-benchmarks --workspace -p frame-support
//! ```
//!
//! The output reveals that there are some dependencies that expose the feature but don't get it
//! passed down:
//!
//! ```pre
//! crate "frame-support"
//!   feature "runtime-benchmarks"
//!     must propagate to:
//!       frame-system
//!       sp-runtime
//!       sp-staking
//! Generated 1 errors and 0 warnings and fixed 0 issues.
//! ```
//!
//! Without the `-p` it will detect many more problems. You can verify this for the [frame-support](https://github.com/paritytech/substrate/blob/ce2cee35f8f0fc5968ea6ffaffa6660dcd008804/frame/support/Cargo.toml#L71) which is indeed missing the feature for `sp-runtime` while that is clearly [sp-runtime](https://github.com/paritytech/substrate/blob/0b6aec52a90870c999856cd37f7d04789cdd8dfc/primitives/runtime/Cargo.toml#L43) it 🤔.
//!
//! This can be fixed by applying the `--fix` flag like:  
//!
//! ```bash
//! feature lint propagate-feature --manifest-path ../substrate/Cargo.toml --feature runtime-benchmarks --workspace -p frame-support --fix
//! ```
//!
//! Which results in this diff:
//!
//! ```patch
//! -runtime-benchmarks = []
//! +runtime-benchmarks = [
//! +       "frame-system/runtime-benchmarks",
//! +       "sp-runtime/runtime-benchmarks",
//! +       "sp-staking/runtime-benchmarks"
//! +]
//! ```
//!
//! The auto-fix is currently a bit coarse, and does not check for optional dependencies. It will
//! also not add the feature to crates that do not have it, but need it because of a dependency.
//! This will be fixed soon.
//!
//! ## Example - Dependency tracing
//!
//! Recently there was a build error in the [Substrate](https://github.com/paritytech/substrate) master CI which was caused by a downstream dependency [`snow`](https://github.com/mcginty/snow/issues/146). To investigate this, it is useful to see *how* Substrate depends on it.  
//!
//! Let's find out how `node-cli` depends on `snow`:
//!
//! ```bash
//! feature trace --manifest-path substrate/Cargo.toml node-cli snow
//! ```
//!
//! output:
//!
//! ```nocompile
//! node-cli -> try-runtime-cli -> substrate-rpc-client -> sc-rpc-api ->sc-chain-spec -> sc-telemetry -> libp2p -> libp2p-webrtc -> libp2p-noise -> snow
//! ```
//!
//! So it comes from libp2p, okay. Good to know.

#![allow(dead_code)]

pub mod autofix;
pub mod cmd;
pub mod dag;

pub mod prelude {
	pub use super::{
		dag::{Dag, Path},
		CrateId,
	};
}

/// Unique Id of a Rust crate.
///
/// These come in the form of:
/// `<NAME> <VERSION> (<SOURCE>)`  
/// You can get an idea by using `cargo metadata | jq '.packages' | grep '"id"'`.
pub type CrateId = String;
