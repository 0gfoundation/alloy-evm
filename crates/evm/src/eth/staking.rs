//! 0G staking contract system calls.

use crate::{
    block::{BlockExecutionError, BlockValidationError},
    Evm,
};
use alloc::format;
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
        if entry.amount == 0 {
            continue;
        }

        let data = Bytes::from(
            IStakingContract::slashValidatorCall {
                validatorAddress: entry.address,
                amount: entry.amount_wei(),
            }
            .abi_encode(),
        );

        let res = evm.transact_system_call(SYSTEM_ADDRESS, staking_contract, data).map_err(|e| {
            BlockValidationError::msg(format!(
                "slashValidator failed for validator {}: {e}",
                entry.address
            ))
        })?;
        results.push(res);
    }

    Ok(results)
}
