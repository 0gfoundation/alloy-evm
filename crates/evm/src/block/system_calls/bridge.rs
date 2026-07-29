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
use alloy_primitives::{Address, Bytes};
use revm::context_interface::result::ResultAndState;

/// EIP-7685 request type byte for 0G's cross-chain Bridge messages (private namespace).
///
/// Emitted as the **last** entry of `executionRequests` post-Bridge fork — strictly after the
/// standard 0x00/0x01/0x02 (deposit/withdrawal/consolidation) entries to satisfy EIP-7685's
/// monotonic type-byte ordering required by the bridge schema.
pub const BRIDGE_REQUEST_TYPE: u8 = 0xf0;

fn bridge_call_input(
    spec: &impl EthExecutorSpec,
    timestamp: u64,
    calldata: Option<&Bytes>,
) -> Option<(Address, Bytes)> {
    if !spec.is_bridge_active_at_timestamp(timestamp) {
        return None;
    }

    let target = spec.bridge_contract_address()?;
    let calldata = calldata.filter(|data| !data.is_empty())?.clone();
    Some((target, calldata))
}

/// Appends the private 0G request after all standard EIP-7685 requests.
pub(crate) fn append_bridge_request(
    bridge_active: bool,
    raw_request: Option<&Bytes>,
    requests: &mut alloy_eips::eip7685::Requests,
) {
    if let (true, Some(raw_request)) = (bridge_active, raw_request) {
        requests.push_request_with_type(BRIDGE_REQUEST_TYPE, raw_request.clone());
    }
}

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
/// calling `db.commit(...)`, which also notifies the configured state hook.
///
/// Gate behavior is covered here and end-to-end by the executor integration tests (`bridge_tests`)
/// in the downstream reth repository, which exercise this function through a real EVM.
#[inline]
pub(crate) fn transact_bridge_contract_call<Halt>(
    spec: &impl EthExecutorSpec,
    timestamp: u64,
    calldata: Option<&Bytes>,
    evm: &mut impl Evm<HaltReason = Halt>,
) -> Result<Option<ResultAndState<Halt>>, BlockExecutionError> {
    // Note: an *empty message list* still produces non-empty calldata — ABI encoding of
    // `parkRemoteMessages([])` is the 4-byte selector plus the empty-array head — so
    // post-fork every block runs the park system call exactly once, even with zero messages.
    // Do NOT "optimize" this by skipping empty batches: whether the system call runs at all
    // is consensus-sensitive, and any such change must land atomically in both the block
    // build and block verify paths. A one-sided change immediately forks the state root.
    let (target, calldata) = match bridge_call_input(spec, timestamp, calldata) {
        Some(input) => input,
        None => return Ok(None),
    };

    let res = match evm.transact_system_call(SYSTEM_ADDRESS, target, calldata) {
        Ok(res) => res,
        Err(e) => {
            // Classified as an Internal (not Validation) BlockExecutionError, deliberately
            // unlike the sibling system calls (eip7002/7251 use BlockValidationError::
            // *ContractCall). parkRemoteMessages performs no token calls and cannot revert or
            // halt — a deterministic revert/halt comes back as Ok(res) with a non-success
            // result and is committed+logged by the caller. So reaching this Err arm means a
            // non-deterministic infrastructure fault (e.g. an EVMError::Database), which is a
            // node-local problem, not a defect in the block. Marking it Validation would let a
            // transient local DB error invalidate a block the rest of the network accepts;
            // Internal keeps the failure attributed to this node.
            return Err(BlockExecutionError::msg(format!(
                "0G bridge system call execution failed: {e}"
            )));
        }
    };
    Ok(Some(res))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_hardforks::{EthereumHardfork, EthereumHardforks, ForkCondition};
    use alloy_primitives::{address, bytes};

    #[derive(Debug)]
    struct TestSpec {
        activation_timestamp: u64,
        bridge_address: Option<Address>,
    }

    impl EthereumHardforks for TestSpec {
        fn ethereum_fork_activation(&self, _fork: EthereumHardfork) -> ForkCondition {
            ForkCondition::Never
        }
    }

    impl EthExecutorSpec for TestSpec {
        fn deposit_contract_address(&self) -> Option<Address> {
            None
        }

        fn staking_contract_address(&self) -> Option<Address> {
            None
        }

        fn is_staking_activate_at_timestamp(&self, _timestamp: u64) -> bool {
            false
        }

        fn bridge_contract_address(&self) -> Option<Address> {
            self.bridge_address
        }

        fn is_bridge_active_at_timestamp(&self, timestamp: u64) -> bool {
            timestamp >= self.activation_timestamp
        }
    }

    #[test]
    fn bridge_call_input_requires_all_gates() {
        let bridge = address!("000000000000000000000000000000000000b123");
        let calldata = bytes!("01020304");
        let active = TestSpec { activation_timestamp: 10, bridge_address: Some(bridge) };

        assert_eq!(bridge_call_input(&active, 9, Some(&calldata)), None);
        assert_eq!(bridge_call_input(&active, 10, None), None);
        assert_eq!(bridge_call_input(&active, 10, Some(&Bytes::new())), None);

        let disabled = TestSpec { activation_timestamp: 10, bridge_address: None };
        assert_eq!(bridge_call_input(&disabled, 10, Some(&calldata)), None);
        assert_eq!(bridge_call_input(&active, 10, Some(&calldata)), Some((bridge, calldata)));
    }

    #[test]
    fn bridge_request_is_last_and_preserves_raw_bytes() {
        let raw = bytes!("aabbcc");
        let mut requests = alloy_eips::eip7685::Requests::default();
        requests.push_request_with_type(0x02, bytes!("01"));

        append_bridge_request(true, Some(&raw), &mut requests);

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1], bytes!("f0aabbcc"));
    }

    #[test]
    fn inactive_bridge_does_not_append_request() {
        let mut requests = alloy_eips::eip7685::Requests::default();
        append_bridge_request(false, Some(&bytes!("aabb")), &mut requests);
        assert!(requests.is_empty());
    }
}
