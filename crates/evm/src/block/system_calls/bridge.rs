//! 0G Bridge inbound message system call.
//!
//! This is the EL-side counterpart to the EIP-7685 request type `0xf0` carried on the engine
//! API in 0G's private namespace. The CL beacon block emits a list of `BridgeMessage` items
//! as SSZ bytes; the EL decodes them into ABI calldata for
//! `Bridge.parkRemoteMessages(InboundMessage[])` and invokes the Bridge proxy contract
//! from the canonical [`SYSTEM_ADDRESS`] just like EIP-4788/7002. The call only *parks* each
//! message into pending storage — it moves no tokens, charges no fee, and emits no events;
//! actual delivery happens later in a permissionless `deliver` / `deliverBatch` user transaction.
//!
//! Activation is gated by [`EthExecutorSpec::is_bridge_active_at_timestamp`] and a configured
//! [`EthExecutorSpec::bridge_contract_address`]. The hook is a no-op (returns `Ok(None)`) when
//! either gate is closed, when no calldata was attached to the block context, or when the
//! attached calldata is empty.

use crate::{block::BlockExecutionError, eth::spec::EthExecutorSpec, Evm};
use alloc::format;
use alloy_eips::eip7002::SYSTEM_ADDRESS;
use alloy_primitives::Bytes;
use revm::context_interface::result::ResultAndState;

/// EIP-7685 request type byte for 0G's cross-chain Bridge messages (private namespace).
///
/// Emitted as the **last** entry of `executionRequests` post-Bridge fork — strictly after the
/// standard 0x00/0x01/0x02 (deposit/withdrawal/consolidation) entries to satisfy EIP-7685's
/// monotonic type-byte ordering required by the bridge schema.
pub const BRIDGE_REQUEST_TYPE: u8 = 0xf0;

/// Invokes `Bridge.parkRemoteMessages(InboundMessage[])` from [`SYSTEM_ADDRESS`].
///
/// Returns `Ok(None)` (no-op) if any of the following gates is closed:
///   * Bridge fork not active at `timestamp` (`spec.is_bridge_active_at_timestamp`)
///   * Chain spec has no configured bridge contract address
///   * No calldata was attached to the block execution context
///   * Attached calldata is empty
///
/// On the active path the call uses the standard `transact_system_call` semantics: 30M gas,
/// no fee charged, no coinbase reward, executed against the same EVM state as block transactions.
/// The state delta is **not** committed by this function — the caller is responsible for
/// wiring the result into [`SystemCaller::on_state`] and calling `db.commit(...)`.
///
/// Gate behavior is covered end-to-end by the executor integration tests (`bridge_tests`) in
/// the downstream reth repository, which exercise this function through a real EVM. This crate
/// intentionally does not mirror the gate logic in unit tests, as such mirrors drift silently.
#[inline]
pub(crate) fn transact_bridge_contract_call<Halt>(
    spec: &impl EthExecutorSpec,
    timestamp: u64,
    calldata: Option<&Bytes>,
    evm: &mut impl Evm<HaltReason = Halt>,
) -> Result<Option<ResultAndState<Halt>>, BlockExecutionError> {
    // Gate 1: fork-version timestamp
    if !spec.is_bridge_active_at_timestamp(timestamp) {
        return Ok(None);
    }

    // Gate 2: configured contract address
    let target = match spec.bridge_contract_address() {
        Some(addr) => addr,
        None => return Ok(None),
    };

    // Gate 3 + 4: non-empty calldata supplied by the engine API
    //
    // Note: an *empty message list* still produces non-empty calldata — ABI encoding of
    // `parkRemoteMessages([])` is the 4-byte selector plus the empty-array head — so
    // post-fork every block runs the park system call exactly once, even with zero messages.
    // Do NOT "optimize" this by skipping empty batches: whether the system call runs at all
    // is consensus-sensitive, and any such change must land atomically in both the block
    // build and block verify paths. A one-sided change immediately forks the state root.
    let cd = match calldata {
        Some(b) if !b.is_empty() => b.clone(),
        _ => return Ok(None),
    };

    let res = match evm.transact_system_call(SYSTEM_ADDRESS, target, cd) {
        Ok(res) => res,
        Err(e) => {
            return Err(BlockExecutionError::msg(format!(
                "0G bridge system call execution failed: {e}"
            )));
        }
    };
    Ok(Some(res))
}
