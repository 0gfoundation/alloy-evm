//! Abstraction over receipt building logic to allow plugging different primitive types into
//! [`super::EthBlockExecutor`].

use crate::Evm;
use alloy_consensus::{Eip658Value, ReceiptEnvelope, TxEnvelope, TxType};
use revm::{
    context::result::{ExecutionResult, InvalidTransaction},
    state::EvmState,
};

/// Context for building a receipt.
#[derive(Debug)]
pub struct ReceiptBuilderCtx<'a, T, E: Evm> {
    /// Transaction
    pub tx: &'a T,
    /// Reference to EVM. State changes should not be committed to inner database when building
    /// receipt so that [`ReceiptBuilder`] can use data from state before transaction execution.
    pub evm: &'a E,
    /// Result of transaction execution.
    pub result: ExecutionResult<E::HaltReason>,
    /// Reference to EVM state after execution.
    pub state: &'a EvmState,
    /// Cumulative gas used.
    pub cumulative_gas_used: u64,
}

/// Type that knows how to build a receipt based on execution result.
#[auto_impl::auto_impl(&, Arc)]
pub trait ReceiptBuilder {
    /// Transaction type.
    type Transaction;
    /// Receipt type.
    type Receipt;

    /// Builds a receipt given a transaction and the result of the execution.
    fn build_receipt<E: Evm>(
        &self,
        ctx: ReceiptBuilderCtx<'_, Self::Transaction, E>,
    ) -> Self::Receipt;

    /// Builds the synthetic receipt for a transaction that was invalid at final execution
    /// state and is therefore applied as a protocol-level no-op. Used only by executors
    /// running with the invalid-transaction skip enabled, where the block's transaction
    /// list is fixed by consensus before execution and cannot drop members.
    ///
    /// Returning `None` (the default) means this receipt type does not support synthetic
    /// skip receipts and the executor must propagate the validation error instead.
    fn build_skipped_receipt(
        &self,
        tx: &Self::Transaction,
        cumulative_gas_used: u64,
        error: &InvalidTransaction,
    ) -> Option<Self::Receipt> {
        let _ = (tx, cumulative_gas_used, error);
        None
    }
}

/// Receipt builder operating on Alloy types.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct AlloyReceiptBuilder;

impl ReceiptBuilder for AlloyReceiptBuilder {
    type Transaction = TxEnvelope;
    type Receipt = ReceiptEnvelope;

    fn build_receipt<E: Evm>(&self, ctx: ReceiptBuilderCtx<'_, TxEnvelope, E>) -> Self::Receipt {
        let receipt = alloy_consensus::Receipt {
            status: Eip658Value::Eip658(ctx.result.is_success()),
            cumulative_gas_used: ctx.cumulative_gas_used,
            logs: ctx.result.into_logs(),
        }
        .with_bloom();

        match ctx.tx.tx_type() {
            TxType::Legacy => ReceiptEnvelope::Legacy(receipt),
            TxType::Eip2930 => ReceiptEnvelope::Eip2930(receipt),
            TxType::Eip1559 => ReceiptEnvelope::Eip1559(receipt),
            TxType::Eip4844 => ReceiptEnvelope::Eip4844(receipt),
            TxType::Eip7702 => ReceiptEnvelope::Eip7702(receipt),
        }
    }
}
