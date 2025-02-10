use alloy_consensus::{Block, BlockBody, Header, TxEnvelope};
use alloy_network::{AnyRpcBlock, AnyRpcHeader};
use alloy_rpc_types::{eth::Block as RpcBlock, ConversionError};
use reth_ethereum_primitives::Transaction;
use reth_primitives::{EthPrimitives, NodePrimitives, TransactionSigned};

use crate::io::BincodeSerializableBlock;

pub trait FromAny: NodePrimitives {
    fn try_from_rpc_block(block: AnyRpcBlock) -> Result<Self::Block, ConversionError>;

    fn try_from_rpc_header(header: AnyRpcHeader) -> Result<Header, ConversionError>;

    fn from_input_block(block: BincodeSerializableBlock<Self>) -> Self::Block;

    fn into_input_block(block: Self::Block) -> BincodeSerializableBlock<Self>;
}

impl FromAny for EthPrimitives {
    fn try_from_rpc_block(block: AnyRpcBlock) -> Result<Block<TransactionSigned>, ConversionError> {
        let RpcBlock { header, transactions, withdrawals, .. } = block.inner;

        let header = Self::try_from_rpc_header(header)?;

        let transactions = transactions
            .try_map(|t| t.inner.inner.try_into_envelope())
            .map_err(|_| ConversionError::Custom("Failed to convert to envelope".to_string()))?
            .try_map(|envelope| {
                let transaction = match &envelope {
                    TxEnvelope::Legacy(signed) => Transaction::Legacy(signed.tx().clone()),
                    TxEnvelope::Eip2930(signed) => Transaction::Eip2930(signed.tx().clone()),
                    TxEnvelope::Eip1559(signed) => Transaction::Eip1559(signed.tx().clone()),
                    TxEnvelope::Eip4844(signed) => Transaction::Eip4844(signed.tx().clone().into()),
                    TxEnvelope::Eip7702(signed) => Transaction::Eip7702(signed.tx().clone()),
                };

                Ok(TransactionSigned::new(transaction, *envelope.signature(), *envelope.tx_hash()))
            })?
            .into_transactions_vec();

        let body = BlockBody { transactions, ommers: vec![], withdrawals };

        Ok(Block { header, body })
    }

    fn try_from_rpc_header(header: AnyRpcHeader) -> Result<Header, ConversionError> {
        header.inner.try_into_header().map_err(|_| {
            ConversionError::Custom("Failed to convert header from RPC type".to_string())
        })
    }

    fn from_input_block(block: BincodeSerializableBlock<Self>) -> Block<TransactionSigned> {
        Block { header: block.header, body: block.body }
    }

    fn into_input_block(block: Self::Block) -> BincodeSerializableBlock<Self> {
        BincodeSerializableBlock { header: block.header, body: block.body }
    }
}

//impl FromAnyRpcBlock for OpPrimitives {
//    fn from(block: &AnyRpcBlock) -> Self::Block {
//        todo!()
//    }
//}
