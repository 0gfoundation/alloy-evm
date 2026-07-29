//! 0G staking contract system calls.

use crate::{
    block::{BlockExecutionError, BlockValidationError},
    Evm,
};
use alloc::{format, vec::Vec};
use alloy_eips::{eip4895::Withdrawal, eip7002::SYSTEM_ADDRESS};
use alloy_primitives::{Address, Bytes};
use alloy_sol_types::{sol, SolCall};
use revm::context_interface::result::ResultAndState;

sol! {
    #[allow(missing_docs)]
    interface IStakingContract {
        function slashValidator(address validatorAddress, uint256 amount) external;
    }
}

fn slash_validator_calldata(entry: &Withdrawal) -> Option<Bytes> {
    if entry.amount == 0 {
        return None;
    }

    Some(Bytes::from(
        IStakingContract::slashValidatorCall {
            validatorAddress: entry.address,
            amount: entry.amount_wei(),
        }
        .abi_encode(),
    ))
}

/// Applies consensus-layer slash metadata by calling
/// `StakingContract.slashValidator` for each slashed validator entry.
pub fn apply_staking_slashings<E>(
    evm: &mut E,
    slashed: &[Withdrawal],
    staking_contract: Address,
) -> Result<Vec<ResultAndState<E::HaltReason>>, BlockExecutionError>
where
    E: Evm,
{
    let mut results = Vec::with_capacity(slashed.len());

    for entry in slashed {
        let Some(data) = slash_validator_calldata(entry) else {
            continue;
        };

        let res =
            evm.transact_system_call(SYSTEM_ADDRESS, staking_contract, data).map_err(|e| {
                BlockValidationError::msg(format!(
                    "slashValidator failed for validator {}: {e}",
                    entry.address
                ))
            })?;
        results.push(res);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn zero_amount_slashing_is_ignored() {
        assert_eq!(slash_validator_calldata(&Withdrawal::default()), None);
    }

    #[test]
    fn slashing_calldata_preserves_validator_and_amount() {
        let entry = Withdrawal {
            address: address!("1234567890123456789012345678901234567890"),
            amount: 42,
            ..Default::default()
        };

        let calldata = slash_validator_calldata(&entry).unwrap();
        assert_eq!(&calldata[..4], IStakingContract::slashValidatorCall::SELECTOR);
        assert_eq!(&calldata[16..36], entry.address.as_slice());
        assert_eq!(&calldata[36..], entry.amount_wei().to_be_bytes::<32>());
    }
}
