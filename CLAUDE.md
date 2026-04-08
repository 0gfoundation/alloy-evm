# CLAUDE.md - alloy-evm (0G Fork)

## Project Overview

This is the **0G Foundation fork** of [alloy-evm](https://github.com/alloy-rs/evm), an EVM abstraction layer on top of [revm](https://github.com/bluealloy/revm). The upstream library provides common implementations of EVMs used by Reth and other Rust-based Ethereum execution clients.

- **Origin**: `git@github.com:0gfoundation/alloy-evm.git` (remote `origin`)
- **Upstream**: `https://github.com/alloy-rs/evm.git` (remote `upstream`)
- **Base version**: v0.20.1 (upstream, on `main`); `dev` branch is based on v0.21.1 with 0G modifications

### Branch Structure

- `main` - Tracks upstream v0.20.1 (closest to upstream)
- `dev` - **Primary 0G branch** (this is what `0g-reth` depends on via git rev `8b7799f`). Contains all 0G-specific features: stateful precompiles, minimum gas enforcement, staking distribution, and staking spec extensions
- `mainnet-staking` - Intermediate branch with staking features but without the latest refinements in `dev`
- `precompiles` - Feature branch for stateful precompiles support
- `dev2` - Intermediate development branch for stateful precompiles

## Key 0G-Specific Modifications

### 1. Stateful Precompiles (`crates/evm/src/precompiles.rs`)

The most significant change. Upstream alloy-evm only supports stateless precompiles (pure functions of input). 0G adds support for **stateful precompiles** that can read/write EVM state during execution.

Changes to `PrecompilesMap` and `DynPrecompiles`:
- `DynPrecompiles.inner` renamed to `DynPrecompiles.stateless` (HashMap of address -> DynPrecompile)
- New `DynPrecompiles.stateful` field (HashSet of addresses for stateful precompiles)
- `get()` renamed to `get_stateless()` to only look up stateless precompiles
- New `is_stateful()` method to check if an address is a stateful precompile
- `PrecompileProvider::run()` modified to dispatch to `run_stateful_precompile()` from the forked revm when the address is stateful
- `PrecompileProvider::contains()` checks both stateless and stateful sets
- `ensure_dynamic_precompiles()` converts both stateless and stateful precompiles from the static set

This depends on the **0G fork of revm** (`0gfoundation/revm`) which adds `stateful_precompiles::run_stateful_precompile`, `Precompiles::stateless()`, `Precompiles::stateful()`, and `Precompiles::is_stateful()`.

### 2. Minimum Gas Used Enforcement (`crates/evm/src/eth/block.rs`)

In `execute_transaction()`, enforces that each transaction uses at least **80% of its gas limit**:
- If actual gas used < 80% of tx gas limit, the reported gas_used is set to 80%
- Note: In the `dev` branch, the extra cost deduction from sender/coinbase balance was **removed** (present in `mainnet-staking` but reverted in `dev`), keeping only the gas_used floor

### 3. Staking Distribution via Withdrawals (`crates/evm/src/eth/block.rs`)

In `finish_block()`, after processing balance increments:
- Checks if withdrawals encode a staking distribution (sentinel: `withdrawals[0].validator_index == u64::MAX` and `len > 1`)
- Calls the staking contract via `transact_system_call()` using `eip7002::SYSTEM_ADDRESS` as caller
- Contract address is determined by `EthExecutorSpec::staking_contract_address()` with fallback to `0xea224dBB52F57752044c0C86aD50930091F561B9`
- Commits the resulting state changes and notifies via `StateChangePostBlockSource::StakingDistribution`

### 4. Staking Spec Extensions (`crates/evm/src/eth/spec.rs`, `crates/evm/src/block/state_hook.rs`)

- `EthExecutorSpec` trait extended with:
  - `staking_contract_address() -> Option<Address>`
  - `is_staking_activate_at_timestamp(timestamp: u64) -> bool`
- `EthSpec` default implementation returns `None`/`false` (0g-reth overrides these)
- New `StateChangePostBlockSource::StakingDistribution` variant added to state hook enum
- `EthBlockExecutionCtx` extended with `timestamp: u64` field

### 5. Spec ID Modules (`crates/evm/src/spec.rs`, `crates/op-evm/src/spec.rs`)

Moved from `crates/evm/src/eth/spec_id.rs` and `crates/evm/src/op/spec_id.rs` to top-level modules. Contains Ethereum hardfork -> SpecId and OP hardfork -> OpSpecId mapping functions. The `op` module was removed entirely (env and spec_id moved or deleted).

### 6. Removed `PartialEq + Eq` from `EvmEnv` (`crates/evm/src/env.rs`)

`EvmEnv` derive changed from `#[derive(Debug, Clone, Default, PartialEq, Eq)]` to `#[derive(Debug, Clone, Default)]` to accommodate fields that don't implement equality.

## Dependency on 0G revm Fork

The `[patch.crates-io]` in `Cargo.toml` (on `dev`/`mainnet-staking` branches) redirects revm dependencies:

```toml
[patch.crates-io]
revm = { git = "https://github.com/0gfoundation/revm", rev = "71cbb675..." }
op-revm = { git = "https://github.com/0gfoundation/revm", rev = "71cbb675..." }
```

The local revm repo at `/Users/wangfan/Project/0g/revm` is the same fork. Commented-out path overrides exist for local development:
```toml
# revm = { path = "../revm/crates/revm" }
# op-revm = { path = "../revm/crates/op-revm" }
```

## How 0g-reth Depends on This

`0g-reth` (at `/Users/wangfan/Project/0g/0g-reth`) depends on the `dev` branch via:

```toml
# In 0g-reth Cargo.toml [patch.crates-io]
alloy-evm = { git = "https://github.com/0gfoundation/alloy-evm", rev = "8b7799f..." }
```

This rev corresponds to "Merge pull request #9 from 0gfoundation/pick-geth-features" on the `dev` branch. `0g-reth` uses alloy-evm extensively across: `reth-ethereum-evm`, `reth-evm`, `reth-chainspec`, `reth-optimism-evm`, `reth-rpc-eth-api`, payload builders, and many examples.

## Crate Structure

### `crates/evm` (`alloy-evm`)
Core EVM abstraction crate. Key modules:
- `evm.rs` - Core `Evm`, `EvmFactory`, `EvmExt` traits
- `eth/` - Ethereum-specific implementation (`EthEvm`, `EthEvmFactory`, `EthBlockExecutor`)
  - `block.rs` - Block execution with 0G gas/staking modifications
  - `spec.rs` - `EthExecutorSpec` trait with 0G staking extensions
  - `eip6110.rs` - Deposit parsing
  - `dao_fork.rs` - DAO fork handling
- `block/` - Block execution abstractions (`BlockExecutor`, errors, state hooks, system calls)
- `precompiles.rs` - `PrecompilesMap` with 0G stateful precompile support
- `traits.rs` - `EvmInternals` for precompile state access
- `env.rs` - `EvmEnv` container type
- `tx.rs` - Transaction environment conversion traits
- `spec.rs` - Ethereum hardfork -> SpecId mapping (moved from eth/spec_id.rs)
- `tracing.rs` - Transaction tracer
- `overrides.rs` - State/block overrides (behind `overrides` feature)
- `call.rs` - Call utilities (behind `call-util` feature)

Features: `std` (default), `secp256k1`, `op` (Optimism support), `overrides`, `call-util`

### `crates/op-evm` (`alloy-op-evm`)
Optimism EVM implementation. Key modules:
- `lib.rs` - `OpEvm` and `OpEvmFactory`
- `block/` - `OpBlockExecutor` and `OpBlockExecutorFactory`
- `spec.rs` - OP hardfork -> OpSpecId mapping (moved from evm/op/spec_id.rs)

## Build and Test Commands

```bash
# Build all crates
cargo build

# Build with all features
cargo build --all-features

# Run tests
cargo test

# Run tests for specific crate
cargo test -p alloy-evm
cargo test -p alloy-op-evm

# Check no_std compatibility
./scripts/check_no_std.sh

# Clippy
cargo clippy --all-targets --all-features

# Format check
cargo fmt --check
```

### Local Development with revm

To develop against the local revm fork, uncomment the path overrides in `Cargo.toml`:
```toml
revm = { path = "../revm/crates/revm" }
op-revm = { path = "../revm/crates/op-revm" }
```

## Dependency Chain

```
0g-reth --> alloy-evm (this repo, 0G fork) --> revm (0G fork at 0gfoundation/revm)
```

All three repos are 0G forks with coordinated modifications for stateful precompile support and staking features.
